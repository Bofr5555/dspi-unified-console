// -- WP QP12 -- Control surfaces (CsController.h).
#include "CsController.h"

#include "Core.h"
#include "DeviceModel.h"
#include "SettingsController.h"
#include "Snapshot.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QTimer>
#include <algorithm>
#include <cmath>
#include <memory>

static const Registrar::Controller<CsController> csController;

// The four Control pages, with the Mac's ids and orders (SettingsController.h)
static const SettingsRegistry::Page surfacesPage{
    "surfaces", SettingsRegistry::Control, 10, QT_TRANSLATE_NOOP("SettingsPages", "Control Surfaces"),
    "sliders-horizontal", "SurfacesPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Push Button"), QT_TRANSLATE_NOOP("SettingsPages", "Toggle Switch"),
      QT_TRANSLATE_NOOP("SettingsPages", "Potentiometer"), QT_TRANSLATE_NOOP("SettingsPages", "Fader"),
      QT_TRANSLATE_NOOP("SettingsPages", "Rotary Encoder"), QT_TRANSLATE_NOOP("SettingsPages", "Indicator LED"),
      QT_TRANSLATE_NOOP("SettingsPages", "IR Remote"), QT_TRANSLATE_NOOP("SettingsPages", "Remote Buttons"),
      QT_TRANSLATE_NOOP("SettingsPages", "Learn"), QT_TRANSLATE_NOOP("SettingsPages", "Display"),
      QT_TRANSLATE_NOOP("SettingsPages", "Dashboard Pages"), QT_TRANSLATE_NOOP("SettingsPages", "GPIO"),
      QT_TRANSLATE_NOOP("SettingsPages", "pin"), QT_TRANSLATE_NOOP("SettingsPages", "knob") } };
static const SettingsRegistry::Page groupsPage{
    "groups", SettingsRegistry::Control, 30, QT_TRANSLATE_NOOP("SettingsPages", "Channel Groups"), "group",
    "GroupsPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Members"), QT_TRANSLATE_NOOP("SettingsPages", "Channel Type"),
      QT_TRANSLATE_NOOP("SettingsPages", "zone"), QT_TRANSLATE_NOOP("SettingsPages", "stereo pair") } };
static const SettingsRegistry::Page macrosPage{
    "macros", SettingsRegistry::Control, 40, QT_TRANSLATE_NOOP("SettingsPages", "Macros"), "list-ordered",
    "MacrosPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Step"), QT_TRANSLATE_NOOP("SettingsPages", "Wait Before"),
      QT_TRANSLATE_NOOP("SettingsPages", "sequence") } };
static const SettingsRegistry::Page auxPage{
    "aux", SettingsRegistry::Control, 50, QT_TRANSLATE_NOOP("SettingsPages", "Auxiliary Outputs"), "power",
    "AuxPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "On/Off Output"), QT_TRANSLATE_NOOP("SettingsPages", "Dimmable Output"),
      QT_TRANSLATE_NOOP("SettingsPages", "relay"), QT_TRANSLATE_NOOP("SettingsPages", "trigger"),
      QT_TRANSLATE_NOOP("SettingsPages", "boot") } };

namespace {
constexpr int kTargetOutputs = 2;        // CS_TARGET_OUTPUT_CH
constexpr int kStatusNoIr = 0x1E;        // CS_STATUS_NO_IR
constexpr int kLearnArmed = 1;           // CS_IR_LEARN_STATE_ARMED
constexpr int kLearnDone = 2;
constexpr int kLearnTimeout = 3;
constexpr int kLearnPolls = 115;         // 100 ms apart: the device's 10 s window and a margin
constexpr int kMacroNone = 0xFF;         // CS_MACRO_NONE
constexpr int kMacroPolls = 120;         // 250 ms apart, then the badge is left as it is
constexpr int kPageActive = 0x01;        // CS_DPAGE_ACTIVE
constexpr uint32_t kReplyBytes = 16384;

QByteArray compact(const QJsonObject &o) { return QJsonDocument(o).toJson(QJsonDocument::Compact); }
QJsonObject parse(const QByteArray &text) { return QJsonDocument::fromJson(text).object(); }

// Worker thread: the configuration as JSON
QByteArray readCs(FfiCore *c) {
    return readSized([c](char *b, uint32_t n) { return dspi_cs_snapshot(c, b, n); }, 1u << 16);
}

// Worker thread: one operation, sent once
QJsonObject applyOnce(FfiCore *c, const QJsonObject &op) {
    const QByteArray text = compact(op);
    QByteArray buf(int(kReplyBytes), '\0');
    const uint32_t need = dspi_cs_apply(c, text.constData(), buf.data(), kReplyBytes);
    if (need == 0 || need > kReplyBytes) return QJsonObject{ { QStringLiteral("ok"), false } };
    buf.truncate(int(need) - 1);
    return parse(buf);
}

// Worker thread: what an editor shows for a draft, or {} on a refusal
QJsonObject describeDraft(FfiCore *c, const QString &kind, int slot, const QJsonObject &draft) {
    const QByteArray k = kind.toUtf8(), d = compact(draft);
    const QJsonObject o = parse(readSized([&](char *b, uint32_t n) {
        return dspi_cs_describe(c, k.constData(), uint32_t(slot), d.constData(), b, n);
    }));
    return o.value(QLatin1String("ok")).toBool() ? o : QJsonObject();
}

// Worker thread: the draft after one change: {ok, draft} or {ok: false, error}
QJsonObject editDraft(FfiCore *c, const QString &kind, int slot, const QJsonObject &draft, const QString &row,
                      const QVariant &value) {
    QJsonObject change{ { QStringLiteral("id"), row } };
    change.insert(QStringLiteral("value"), value.typeId() == QMetaType::Bool ? QJsonValue(value.toBool())
                                                                            : QJsonValue(value.toDouble()));
    const QByteArray k = kind.toUtf8(), d = compact(draft), ch = compact(change);
    return parse(readSized([&](char *b, uint32_t n) {
        return dspi_cs_edit(c, k.constData(), uint32_t(slot), d.constData(), ch.constData(), b, n);
    }));
}

QString failureOf(FfiCore *c, const QJsonObject &reply) {
    if (reply.contains(QLatin1String("error"))) return reply.value(QLatin1String("error")).toString();
    const int code = reply.value(QLatin1String("status")).toInt();
    const QString text = QString::fromUtf8(readSized([c, code](char *b, uint32_t n) {
        return dspi_cs_status_text(c, uint8_t(code), b, n);
    }));
    return text.isEmpty() ? CsController::tr("Failed to apply the binding") : text;
}

QString protocolName(int protocol) {
    switch (protocol) {
    case 1: return QStringLiteral("NEC");
    case 2: return QStringLiteral("RC5");
    case 3: return QStringLiteral("RC6");
    case 4: return CsController::tr("Generic");
    default: return CsController::tr("None");
    }
}

// Seconds in the most readable unit (the Mac's fmtDelaySeconds)
QString delayText(double s) {
    if (s < 60) return s < 10 ? CsController::tr("%1 s").arg(QString::number(s, 'g', 2))
                              : CsController::tr("%1 s").arg(QString::number(s, 'f', 0));
    const double mins = s / 60;
    return CsController::tr("%1 min").arg(QString::number(mins, 'f', mins == std::round(mins) ? 0 : 1));
}

QVariantList rowsOf(const QJsonObject &description, bool dropType) {
    QVariantList out;
    for (const QJsonValue &v : description.value(QLatin1String("rows")).toArray()) {
        const QJsonObject row = v.toObject();
        if (dropType && row.value(QLatin1String("id")).toString() == QLatin1String("type")) continue;
        out.append(row.toVariantMap());
    }
    return out;
}

QJsonObject rowById(const QJsonObject &description, const char *id) {
    for (const QJsonValue &v : description.value(QLatin1String("rows")).toArray())
        if (v.toObject().value(QLatin1String("id")).toString() == QLatin1String(id)) return v.toObject();
    return {};
}

int num(const QJsonObject &o, const char *key) { return o.value(QLatin1String(key)).toInt(); }

QString keyOf(const QString &kind, int slot) { return kind + QLatin1Char('/') + QString::number(slot); }
} // namespace

CsController::CsController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(m_ctx.device, &DeviceModel::snapshotApplied, this, &CsController::onSnapshot);
    connect(m_ctx.device, &DeviceModel::stateChanged, this, &CsController::onNotify);

    // The status codes in words, once: the core needs no device for them
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            QHash<int, QString> texts;
            for (int code = 1; code < 0x40; ++code)
                texts.insert(code, QString::fromUtf8(readSized([c, code](char *b, uint32_t n) {
                                 return dspi_cs_status_text(c, uint8_t(code), b, n);
                             })));
            return texts;
        },
        [this](const QHash<int, QString> &texts) {
            m_statusTexts = texts;
            bump();
        });

    // The learn window: its result is read every 100 ms while it is armed
    m_learnPoll = m_ctx.poller->add(
        QStringLiteral("control surface learn"), 100, this,
        [](FfiCore *c) { return applyOnce(c, QJsonObject{ { QStringLiteral("op"), QStringLiteral("learnRead") } }); },
        [this](const QJsonObject &r) {
            if (m_learningSub < 0) return;
            ++m_learnTicks;
            if (r.value(QLatin1String("ok")).toBool() && num(r, "state") != kLearnArmed) finishLearn(r, false);
            else if (!r.value(QLatin1String("ok")).toBool() || m_learnTicks >= kLearnPolls) finishLearn(r, true);
        });
    m_ctx.poller->setEnabled(m_learnPoll, false);

    // A running macro: nothing pushes its progress, so it is followed
    m_macroPoll = m_ctx.poller->add(
        QStringLiteral("control surface macro"), 250, this,
        [](FfiCore *c) {
            applyOnce(c, QJsonObject{ { QStringLiteral("op"), QStringLiteral("refreshStatus") } });
            return readCs(c);
        },
        [this](const QByteArray &text) {
            applySnapshot(text);
            const int running = m_snap.value(QLatin1String("ext")).toObject().value(QLatin1String("macroRunning")).toInt(kMacroNone);
            if (running == kMacroNone || ++m_macroTicks >= kMacroPolls) m_ctx.poller->setEnabled(m_macroPoll, false);
        });
    m_ctx.poller->setEnabled(m_macroPoll, false);

    // The panel's state, while a display card shows it
    m_displayPoll = m_ctx.poller->add(
        QStringLiteral("control surface display"), 1000, this,
        [](FfiCore *c) {
            applyOnce(c, QJsonObject{ { QStringLiteral("op"), QStringLiteral("displayStatus") } });
            return readCs(c);
        },
        [this](const QByteArray &text) { applySnapshot(text); });
    m_ctx.poller->setEnabled(m_displayPoll, false);

    // Not now: other controllers are still being made
    QTimer::singleShot(0, this, &CsController::onSnapshot);
}

CsController::~CsController() {
    if (m_joined)
        if (auto *hw = Registrar::instance<HardwareController>()) hw->removeSaveParticipant(this);
}

// ---- the device ---------------------------------------------------------------------

bool CsController::connected() const { return m_ctx.device->connected() && m_ctx.device->state(); }
bool CsController::writable() const { return connected() && !m_ctx.device->readOnly(); }
bool CsController::supported() const { return m_snap.value(QLatin1String("supported")).toBool(); }
QJsonObject CsController::status() const { return m_snap.value(QLatin1String("status")).toObject(); }
bool CsController::dirty() const { return supported() && status().value(QLatin1String("dirty")).toBool(); }
int CsController::capsVersion() const { return num(m_snap, "capsVersion"); }
int CsController::typeCount() const { return int(m_snap.value(QLatin1String("types")).toArray().size()); }

int CsController::slotCount() const {
    const int n = num(m_snap, "maxBindings");
    return std::min(n > 0 ? n : int(CS_MAX_BINDINGS), int(m_snap.value(QLatin1String("bindings")).toArray().size()));
}
int CsController::groupCount() const {
    return std::min(num(m_snap, "maxGroups"), int(m_snap.value(QLatin1String("groups")).toArray().size()));
}
int CsController::macroCount() const {
    return maxSteps() > 0 ? std::min(num(m_snap, "maxMacros"), int(m_snap.value(QLatin1String("macros")).toArray().size()))
                          : 0;
}
int CsController::maxSteps() const { return std::min(num(m_snap, "maxSteps"), int(CS_MAX_STEPS)); }
int CsController::irCount() const {
    if (typeCount() <= Ir) return 0;
    return std::min(num(m_snap, "maxIr"), int(m_snap.value(QLatin1String("ir")).toArray().size()));
}

void CsController::joinBar() {
    if (m_joined) return;
    if (auto *hw = Registrar::instance<HardwareController>()) {
        m_joined = true;
        hw->addSaveParticipant(this, 40);
    }
}

void CsController::onSnapshot() {
    joinBar();
    const DeviceSnapshot &snap = m_ctx.device->snapshot();
    if (!connected()) {
        if (!m_snap.isEmpty() || !m_records.isEmpty()) clear();
        m_serial.clear();
        m_generation = 0;
        updatePages();
        return;
    }
    // Another device: what was held belongs to the old one. A re-plug of the
    // same unit keeps the drafts and reads the configuration again.
    if (m_ctx.device->serial() != m_serial) {
        clear();
        m_serial = m_ctx.device->serial();
        m_generation = snap.generation;
        fetch();
    } else if (snap.generation != m_generation) {
        m_generation = snap.generation;
        fetch();
    }
}

void CsController::onNotify(uint flags) {
    // An aux output switched by a control, a learn that ended at the device
    if (flags & (NOTIFY_CS_AUX | NOTIFY_IR_LEARN)) readSnapshot();
}

void CsController::clear() {
    ++m_epoch;
    if (m_learningSub >= 0) endLearn();
    m_snap = QJsonObject();
    m_records.clear();
    m_displayDescription = QJsonObject();
    m_pageDescriptions.clear();
    m_loadError.clear();
    m_displayMessage.clear();
    m_learnMessage.clear();
    m_ctx.poller->setEnabled(m_macroPoll, false);
    m_ctx.poller->setEnabled(m_displayPoll, false);
    bump();
}

void CsController::fetch() {
    if (!connected() || m_fetching) return;
    m_fetching = true;
    const int epoch = m_epoch;
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            const QJsonObject r = applyOnce(c, QJsonObject{ { QStringLiteral("op"), QStringLiteral("refresh") } });
            const QString failure = r.value(QLatin1String("ok")).toBool() ? QString() : failureOf(c, r);
            return std::make_pair(failure, readCs(c));
        },
        [this, epoch](const std::pair<QString, QByteArray> &r) {
            m_fetching = false;
            if (epoch != m_epoch) return;
            m_loadError = r.first;
            applySnapshot(r.second);
            updatePages();
            notifyPins();
            bump();
        });
}

void CsController::readSnapshot(std::function<void()> then) {
    if (!connected()) return;
    const int epoch = m_epoch;
    m_ctx.core->worker()->call(this, [](FfiCore *c) { return readCs(c); },
                               [this, epoch, then](const QByteArray &text) {
                                   if (epoch != m_epoch) return;
                                   applySnapshot(text);
                                   if (then) then();
                               });
}

void CsController::applySnapshot(const QByteArray &text) {
    if (text.isEmpty()) return;
    const QJsonObject next = parse(text);
    // A poll that finds nothing new changes nothing
    if (next.isEmpty() || next == m_snap) return;
    m_snap = next;
    syncRecords();
    updatePages();
    describeAll();
    bump();
}

QJsonObject CsController::liveFor(const QString &kind, int slot) const {
    if (kind == QLatin1String("macro")) {
        const QJsonObject m = m_snap.value(QLatin1String("macros")).toArray().at(slot).toObject();
        return QJsonObject{ { QStringLiteral("name"), m.value(QLatin1String("name")).toString() },
                            { QStringLiteral("steps"), m.value(QLatin1String("steps")).toArray() } };
    }
    const char *table = kind == QLatin1String("binding") ? "bindings" : kind == QLatin1String("ir") ? "ir" : "groups";
    return m_snap.value(QLatin1String(table)).toArray().at(slot).toObject();
}

// Every slot the device has gets a record; a draft with no edits follows the
// device (the Mac's re-seed), one being edited or applied stays
void CsController::syncRecords() {
    const auto sync = [this](const QString &kind, int count) {
        for (int i = 0; i < count; ++i) {
            const QString key = keyOf(kind, i);
            const QJsonObject live = liveFor(kind, i);
            auto it = m_records.find(key);
            if (it == m_records.end()) {
                Record r;
                r.kind = kind;
                r.slot = i;
                r.live = live;
                r.draft = live;
                m_records.insert(key, r);
                continue;
            }
            if (!it->applying && !it->editing && it->draft == it->live) it->draft = live;
            it->live = live;
            if (it->nameEdited && it->nameEdit == nameOf(i)) it->nameEdited = false;
        }
    };
    sync(QStringLiteral("binding"), slotCount());
    sync(QStringLiteral("ir"), irCount());
    sync(QStringLiteral("group"), groupCount());
    sync(QStringLiteral("macro"), macroCount());
}

// The rows of every record a page could show, in one worker job
void CsController::describeAll() {
    struct Ask {
        QString key, kind;
        int slot;
        QJsonObject draft;
    };
    QList<Ask> asks;
    for (auto it = m_records.cbegin(); it != m_records.cend(); ++it) {
        const Record &r = it.value();
        if (!inUse(r)) continue;
        if (r.kind == QLatin1String("macro")) {
            const QJsonArray steps = r.draft.value(QLatin1String("steps")).toArray();
            for (int s = 0; s < steps.size(); ++s)
                asks.append({ it.key() + QLatin1Char('/') + QString::number(s), QStringLiteral("step"), 0, steps.at(s).toObject() });
            continue;
        }
        asks.append({ it.key(), r.kind, r.slot, r.draft });
    }
    if (displaySlot() >= 0) {
        const QJsonObject d = m_snap.value(QLatin1String("display")).toObject();
        asks.append({ QStringLiteral("display"), QStringLiteral("display"), 0, d.value(QLatin1String("config")).toObject() });
        const QJsonArray pages = d.value(QLatin1String("pages")).toArray();
        const int maxPages = std::min(num(d, "maxPages"), int(pages.size()));
        for (int i = 0; i < maxPages; ++i)
            if (num(pages.at(i).toObject(), "flags") & kPageActive)
                asks.append({ keyOf(QStringLiteral("page"), i), QStringLiteral("page"), i, pages.at(i).toObject() });
    }
    const int epoch = m_epoch;
    m_ctx.core->worker()->call(
        this,
        [asks](FfiCore *c) {
            QHash<QString, QJsonObject> out;
            for (const Ask &a : asks) out.insert(a.key, describeDraft(c, a.kind, a.slot, a.draft));
            return out;
        },
        [this, epoch](const QHash<QString, QJsonObject> &out) {
            if (epoch != m_epoch) return;
            m_pageDescriptions.clear();
            for (auto it = out.cbegin(); it != out.cend(); ++it) {
                const QString &key = it.key();
                if (key == QLatin1String("display")) {
                    m_displayDescription = it.value();
                } else if (key.startsWith(QLatin1String("page/"))) {
                    m_pageDescriptions.insert(key.mid(5).toInt(), it.value());
                } else if (key.startsWith(QLatin1String("macro/")) && key.count(QLatin1Char('/')) == 2) {
                    const int cut = int(key.lastIndexOf(QLatin1Char('/')));
                    Record *r = rec(key.left(cut));
                    if (!r) continue;
                    const int s = key.mid(cut + 1).toInt();
                    while (r->stepDescriptions.size() <= s) r->stepDescriptions.append(QJsonObject());
                    r->stepDescriptions[s] = it.value();
                } else if (Record *r = rec(key)) {
                    r->description = it.value();
                }
            }
            // Steps the drafts no longer hold
            for (Record &r : m_records)
                if (r.kind == QLatin1String("macro"))
                    while (r.stepDescriptions.size() > r.draft.value(QLatin1String("steps")).toArray().size())
                        r.stepDescriptions.removeLast();
            bump();
        });
}

// One record's rows after its draft changed
void CsController::describeOne(const QString &key) {
    Record *r = rec(key);
    if (!r) return;
    QList<std::pair<int, QJsonObject>> steps;
    if (r->kind == QLatin1String("macro")) {
        const QJsonArray a = r->draft.value(QLatin1String("steps")).toArray();
        for (int s = 0; s < a.size(); ++s) steps.append({ s, a.at(s).toObject() });
    }
    const QString kind = r->kind;
    const int slot = r->slot;
    const QJsonObject draft = r->draft;
    const int epoch = m_epoch;
    m_ctx.core->worker()->call(
        this,
        [kind, slot, draft, steps](FfiCore *c) {
            QJsonArray stepRows;
            for (const auto &s : steps) stepRows.append(describeDraft(c, QStringLiteral("step"), 0, s.second));
            return std::make_pair(steps.isEmpty() && kind != QLatin1String("macro") ? describeDraft(c, kind, slot, draft)
                                                                                    : QJsonObject(),
                                  stepRows);
        },
        [this, key, epoch](const std::pair<QJsonObject, QJsonArray> &d) {
            if (epoch != m_epoch) return;
            Record *r = rec(key);
            if (!r) return;
            if (r->kind == QLatin1String("macro")) r->stepDescriptions = d.second;
            else r->description = d.first;
            bump();
        });
}

void CsController::bump() {
    ++m_revision;
    emit changed();
    const bool nowDirty = dirty();
    if (m_joined && nowDirty != m_wasDirty)
        if (auto *hw = Registrar::instance<HardwareController>()) hw->participantChanged();
    m_wasDirty = nowDirty;
}

void CsController::beginOp() {
    ++m_ops;
    if (auto *hw = Registrar::instance<HardwareController>(); hw && m_joined) hw->participantChanged();
    bump();
}

void CsController::endOp() {
    m_ops = std::max(0, m_ops - 1);
    if (auto *hw = Registrar::instance<HardwareController>(); hw && m_joined) hw->participantChanged();
    bump();
}

void CsController::run(const QString &what, const QList<QJsonObject> &ops, std::function<void(const Reply &)> done) {
    auto reply = std::make_shared<Reply>();
    reply->failure = !connected() ? tr("No device connected.") : tr("The device is read-only.");
    const int epoch = m_epoch;
    beginOp();
    m_ctx.device->write(
        what,
        [ops, reply](FfiCore *c) {
            reply->ran = true;
            reply->ok = true;
            reply->failure.clear();
            for (const QJsonObject &op : ops) {
                reply->json = applyOnce(c, op);
                if (!reply->json.value(QLatin1String("ok")).toBool()) {
                    reply->ok = false;
                    reply->failure = failureOf(c, reply->json);
                    break;
                }
            }
            reply->snapshot = readCs(c);
            return reply->ok;
        },
        [this, reply, done, epoch](bool) {
            endOp();
            // Replaced by a later write of the same name before it started
            if (!reply->ran && writable()) return;
            if (epoch != m_epoch) return;
            applySnapshot(reply->snapshot);
            notifyPins();
            if (done) done(*reply);
        });
}

// The other pages' pin pickers and the Overview's map hold the live bindings' pins
void CsController::notifyPins() {
    // -- WP QP12 -- see the requests in the report: both re-read on a snapshot
    if (auto *s = Registrar::instance<SettingsController>()) s->refreshSurfacePins();
    if (auto *hw = Registrar::instance<HardwareController>()) hw->refreshSurfacePins();
}

void CsController::updatePages() {
    auto *settings = Registrar::instance<SettingsController>();
    if (!settings) return;
    const bool have = loaded();
    // Shown while the device is still being read, as the Mac does
    settings->setPageAvailable(QStringLiteral("surfaces"), !connected() || m_snap.isEmpty() || supported());
    settings->setPageAvailable(QStringLiteral("groups"), have && groupCount() > 0);
    settings->setPageAvailable(QStringLiteral("macros"), have && macroCount() > 0);
    settings->setPageAvailable(QStringLiteral("aux"), have && capsVersion() >= 18 && typeCount() > AuxPwm);
}

// ---- records ------------------------------------------------------------------------

CsController::Record *CsController::rec(const QString &key) {
    auto it = m_records.find(key);
    return it == m_records.end() ? nullptr : &it.value();
}
const CsController::Record *CsController::rec(const QString &key) const {
    auto it = m_records.constFind(key);
    return it == m_records.cend() ? nullptr : &it.value();
}

QString CsController::nameOf(int slot) const {
    return m_snap.value(QLatin1String("names")).toArray().at(slot).toString();
}

bool CsController::slotActive(int slot) const {
    return slot >= 0 && slot < 16 && (num(status(), "activeMask") & (1 << slot)) != 0;
}

QString CsController::statusText(int code) const {
    const QString text = m_statusTexts.value(code);
    return text.isEmpty() ? tr("Failed to apply the binding") : text;
}

bool CsController::irDirty(int sub) const {
    const Record *r = rec(keyOf(QStringLiteral("ir"), sub));
    if (!r || r->draft == r->live) return false;
    // A button not learned yet has nothing to apply
    return num(r->draft, "protocol") != 0 || num(r->live, "protocol") != 0;
}

bool CsController::anyIrDirty() const {
    for (int s = 0; s < irCount(); ++s)
        if (irDirty(s)) return true;
    return false;
}

bool CsController::isDirty(const Record &r) const {
    if (r.kind == QLatin1String("binding")) {
        if (r.draft != r.live || r.nameEdited) return true;
        return num(r.draft, "type") == Ir && anyIrDirty();
    }
    if (r.kind == QLatin1String("ir")) return irDirty(r.slot);
    return r.draft != r.live;
}

bool CsController::inUse(const Record &r) const {
    if (r.kind == QLatin1String("binding")) return num(r.draft, "type") != None || num(r.live, "type") != None;
    if (r.kind == QLatin1String("ir")) {
        bool zero = true;
        for (auto it = r.draft.constBegin(); it != r.draft.constEnd(); ++it)
            if (it.value().toDouble() != 0) zero = false;
        return !zero || num(r.live, "protocol") != 0;
    }
    if (r.kind == QLatin1String("group"))
        return num(r.draft, "targetKind") != 0 || !r.draft.value(QLatin1String("name")).toString().isEmpty() ||
               (num(r.live, "targetKind") != 0 && num(r.live, "memberMask") != 0);
    // A macro
    const auto occupied = [](const QJsonObject &m) {
        return !m.value(QLatin1String("name")).toString().isEmpty() || !m.value(QLatin1String("steps")).toArray().isEmpty();
    };
    return occupied(r.draft) || occupied(r.live);
}

int CsController::receiverSlot() const {
    for (int s = 0; s < slotCount(); ++s) {
        const Record *r = rec(keyOf(QStringLiteral("binding"), s));
        if (r && (num(r->draft, "type") == Ir || num(r->live, "type") == Ir)) return s;
    }
    return -1;
}

int CsController::displaySlot() const {
    for (int s = 0; s < slotCount(); ++s) {
        const Record *r = rec(keyOf(QStringLiteral("binding"), s));
        if (r && num(r->live, "type") == Display) return s;
    }
    return -1;
}

QStringList CsController::keys(const QString &page) const {
    QStringList out;
    const auto list = [&](const QString &kind, int count, const std::function<bool(const Record &)> &keep) {
        for (int i = 0; i < count; ++i) {
            const Record *r = rec(keyOf(kind, i));
            if (r && inUse(*r) && keep(*r)) out.append(keyOf(kind, i));
        }
    };
    if (page == QLatin1String("controls") || page == QLatin1String("aux")) {
        const bool aux = page == QLatin1String("aux");
        list(QStringLiteral("binding"), slotCount(), [&](const Record &r) {
            return (isAux(num(r.draft, "type")) || isAux(num(r.live, "type"))) == aux;
        });
    } else if (page == QLatin1String("groups")) {
        list(QStringLiteral("group"), groupCount(), [](const Record &) { return true; });
    } else if (page == QLatin1String("macros")) {
        list(QStringLiteral("macro"), macroCount(), [](const Record &) { return true; });
    } else if (page == QLatin1String("remotes")) {
        list(QStringLiteral("ir"), irCount(), [](const Record &) { return true; });
    }
    return out;
}

QString CsController::typeName(int type) const {
    switch (type) {
    case None: return tr("None");
    case Button: return tr("Push Button");
    case Switch: return tr("Toggle Switch");
    case Pot: return tr("Potentiometer / Fader");
    case Encoder: return tr("Rotary Encoder");
    case Led: return tr("Indicator LED");
    case LedPwm: return tr("Dimmable LED");
    case Ir: return tr("IR Remote");
    case Display: return tr("Display");
    case AuxOut: return tr("On/Off Output");
    case AuxPwm: return tr("Dimmable Output");
    default: return tr("Type %1").arg(type);
    }
}

// Lucide names for the Mac's SF Symbols
QString CsController::typeIcon(int type) const {
    switch (type) {
    case Button: return QStringLiteral("pointer");
    case Switch: return QStringLiteral("toggle-left");
    case Pot: return QStringLiteral("gauge");
    case Encoder: return QStringLiteral("rotate-cw");
    case Led: return QStringLiteral("lightbulb");
    case LedPwm: return QStringLiteral("sun");
    case Ir: return QStringLiteral("radio");
    case Display: return QStringLiteral("monitor");
    case AuxOut: return QStringLiteral("power");
    case AuxPwm: return QStringLiteral("sun-dim");
    default: return QStringLiteral("gauge");
    }
}

QVariantList CsController::addableTypes(const QString &page) const {
    QVariantList out;
    const bool aux = page == QLatin1String("aux");
    bool displayDrafted = false;
    for (int s = 0; s < slotCount(); ++s)
        if (const Record *r = rec(keyOf(QStringLiteral("binding"), s)); r && num(r->draft, "type") == Display)
            displayDrafted = true;
    for (int t = 1; t < typeCount(); ++t) {
        if (isAux(t) != aux) continue;
        if (t == Ir && (receiverSlot() >= 0 || irCount() == 0)) continue;
        if (t == Display && displayDrafted) continue;
        out.append(QVariantMap{ { QStringLiteral("type"), t }, { QStringLiteral("name"), typeName(t) },
                                { QStringLiteral("icon"), typeIcon(t) } });
    }
    return out;
}

bool CsController::canAdd(const QString &kind) const {
    const auto free = [&](const QString &k, int count) {
        for (int i = 0; i < count; ++i)
            if (const Record *r = rec(keyOf(k, i)); r && !inUse(*r)) return true;
        return false;
    };
    if (!writable()) return false;
    if (kind == QLatin1String("binding")) return free(kind, slotCount());
    if (kind == QLatin1String("ir")) return free(kind, irCount());
    if (kind == QLatin1String("group")) return free(kind, groupCount());
    if (kind == QLatin1String("macro")) return free(kind, macroCount());
    return false;
}

QVariantMap CsController::record(const QString &key) const {
    const Record *r = rec(key);
    if (!r) return {};
    QVariantMap m{ { QStringLiteral("key"), key },
                   { QStringLiteral("kind"), r->kind },
                   { QStringLiteral("slot"), r->slot },
                   { QStringLiteral("dirty"), isDirty(*r) },
                   { QStringLiteral("applying"), r->applying },
                   { QStringLiteral("message"), r->message },
                   { QStringLiteral("isError"), r->isError },
                   { QStringLiteral("summary"), r->description.value(QLatin1String("summary")).toString() } };
    if (r->kind == QLatin1String("binding")) {
        const int type = num(r->draft, "type");
        const bool liveConfigured = num(r->live, "type") != None;
        const QString name = r->nameEdited ? r->nameEdit : nameOf(r->slot);
        const QString summary = r->description.value(QLatin1String("summary")).toString();
        m.insert(QStringLiteral("type"), type);
        m.insert(QStringLiteral("typeName"), typeName(type));
        m.insert(QStringLiteral("icon"), typeIcon(type));
        m.insert(QStringLiteral("name"), name);
        m.insert(QStringLiteral("placeholder"), typeName(type));
        // A custom name hides the component type: the summary restates it
        m.insert(QStringLiteral("summary"), name.isEmpty() ? summary : tr("%1 - %2").arg(typeName(type), summary));
        m.insert(QStringLiteral("rows"), rowsOf(r->description, true));
        m.insert(QStringLiteral("liveConfigured"), liveConfigured);
        const bool active = slotActive(r->slot);
        m.insert(QStringLiteral("active"), active);
        const QString pill = isDirty(*r) ? tr("Pending") : active ? tr("Active") : tr("Inactive");
        m.insert(QStringLiteral("pill"), pill);
        m.insert(QStringLiteral("pillGood"), !isDirty(*r) && active);
        const int health = status().value(QLatin1String("slotStatus")).toArray().at(r->slot).toInt();
        m.insert(QStringLiteral("inactive"), liveConfigured && !active);
        m.insert(QStringLiteral("inactiveReason"),
                 tr("Not running: %1 Reassign the conflicting pin, then apply.").arg(statusText(health).toLower()));
        // The badge menu: the type row's options (an aux slot stays an aux output)
        QVariantList types;
        for (const QJsonValue &o : rowById(r->description, "type").value(QLatin1String("options")).toArray()) {
            const int t = num(o.toObject(), "value");
            types.append(QVariantMap{ { QStringLiteral("type"), t }, { QStringLiteral("name"), typeName(t) },
                                      { QStringLiteral("icon"), typeIcon(t) },
                                      { QStringLiteral("disabled"), o.toObject().value(QLatin1String("disabled")).toBool() } });
        }
        m.insert(QStringLiteral("types"), types);
        if (isAux(type)) {
            const double level = m_snap.value(QLatin1String("auxLevel")).toArray().at(r->slot).toDouble() / 256.0;
            m.insert(QStringLiteral("auxOn"), m_snap.value(QLatin1String("auxState")).toArray().at(r->slot).toVariant().toBool());
            m.insert(QStringLiteral("auxLevel"), level);
            m.insert(QStringLiteral("auxLive"), liveConfigured && active);
        }
        if (type == Ir) {
            m.insert(QStringLiteral("remotes"), keys(QStringLiteral("remotes")));
            m.insert(QStringLiteral("maxRemotes"), irCount());
            m.insert(QStringLiteral("receiverLive"), active);
        }
    } else if (r->kind == QLatin1String("ir")) {
        const int protocol = num(r->draft, "protocol");
        const qint64 code = qint64(r->draft.value(QLatin1String("code")).toDouble());
        m.insert(QStringLiteral("learned"), protocol != 0);
        m.insert(QStringLiteral("codeText"),
                 protocol != 0 ? QStringLiteral("%1 0x%2").arg(protocolName(protocol), QStringLiteral("%1").arg(code, 8, 16, QLatin1Char('0')).toUpper())
                               : tr("Not learned"));
        m.insert(QStringLiteral("rows"), rowsOf(r->description, false));
        m.insert(QStringLiteral("learning"), m_learningSub == r->slot);
        m.insert(QStringLiteral("liveLearned"), num(r->live, "protocol") != 0);
    } else if (r->kind == QLatin1String("group")) {
        const bool configured = num(r->draft, "targetKind") != 0 && num(r->draft, "memberMask") != 0;
        const bool liveConfigured = num(r->live, "targetKind") != 0 && num(r->live, "memberMask") != 0;
        m.insert(QStringLiteral("name"), r->draft.value(QLatin1String("name")).toString());
        m.insert(QStringLiteral("placeholder"), tr("Group %1").arg(r->slot + 1));
        m.insert(QStringLiteral("rows"), rowsOf(r->description, false));
        m.insert(QStringLiteral("configured"), configured);
        m.insert(QStringLiteral("liveConfigured"), liveConfigured);
        const int s = m_snap.value(QLatin1String("ext")).toObject().value(QLatin1String("groupStatus")).toArray().at(r->slot).toInt();
        m.insert(QStringLiteral("warning"), s != 0 && liveConfigured ? statusText(s) : QString());
    } else {
        const QJsonArray steps = r->draft.value(QLatin1String("steps")).toArray();
        const QJsonObject ext = m_snap.value(QLatin1String("ext")).toObject();
        const bool running = num(ext, "macroRunning") == r->slot;
        QVariantList stepList;
        double total = 0;
        for (int s = 0; s < steps.size(); ++s) {
            const QJsonObject d = r->stepDescriptions.at(s).toObject();
            total += rowById(d, "preDelay").value(QLatin1String("value")).toDouble();
            stepList.append(QVariantMap{ { QStringLiteral("summary"), d.value(QLatin1String("summary")).toString() },
                                         { QStringLiteral("rows"), rowsOf(d, false) } });
        }
        QString summary;
        if (running)
            summary = tr("Running - step %1 of %2.").arg(num(ext, "macroStep") + 1).arg(r->live.value(QLatin1String("steps")).toArray().size());
        else if (steps.isEmpty())
            summary = tr("No steps yet.");
        else
        {
            const QString count = steps.size() == 1 ? tr("1 step") : tr("%1 steps").arg(steps.size());
            summary = total > 0 ? tr("%1, %2 total").arg(count, delayText(total)) : count;
        }
        m.insert(QStringLiteral("name"), r->draft.value(QLatin1String("name")).toString());
        m.insert(QStringLiteral("placeholder"), tr("Macro %1").arg(r->slot + 1));
        m.insert(QStringLiteral("summary"), summary);
        m.insert(QStringLiteral("running"), running);
        m.insert(QStringLiteral("steps"), stepList);
        m.insert(QStringLiteral("stepCount"), steps.size());
        m.insert(QStringLiteral("maxSteps"), maxSteps());
        m.insert(QStringLiteral("liveSteps"), r->live.value(QLatin1String("steps")).toArray().size());
        m.insert(QStringLiteral("liveConfigured"), inUse(Record{ r->kind, r->slot, r->live, r->live }));
        const int st = ext.value(QLatin1String("macroStatus")).toArray().at(r->slot).toInt();
        m.insert(QStringLiteral("warning"), st != 0 && !r->live.value(QLatin1String("steps")).toArray().isEmpty() ? statusText(st) : QString());
    }
    return m;
}

// ---- edits --------------------------------------------------------------------------

// A record's edits run one after the other, each on the draft the last one left
void CsController::queueEdit(const QString &key, std::function<void()> job) {
    Record *r = rec(key);
    if (!r) return;
    r->edits.enqueue(std::move(job));
    if (!r->editing) nextEdit(key);
}

void CsController::nextEdit(const QString &key) {
    Record *r = rec(key);
    if (!r) return;
    if (r->edits.isEmpty()) {
        r->editing = false;
        return;
    }
    r->editing = true;
    r->edits.dequeue()();
}

void CsController::edit(const QString &key, const QString &row, const QVariant &value) {
    queueEdit(key, [this, key, row, value]() {
        Record *r = rec(key);
        if (!r) return;
        const QString kind = r->kind;
        const int slot = r->slot;
        const QJsonObject draft = r->draft;
        const int epoch = m_epoch;
        m_ctx.core->worker()->call(
            this,
            [kind, slot, draft, row, value](FfiCore *c) {
                const QJsonObject e = editDraft(c, kind, slot, draft, row, value);
                const QJsonObject next = e.value(QLatin1String("draft")).toObject();
                return std::make_pair(e, e.value(QLatin1String("ok")).toBool() ? describeDraft(c, kind, slot, next) : QJsonObject());
            },
            [this, key, epoch](const std::pair<QJsonObject, QJsonObject> &e) {
                if (epoch != m_epoch) return;
                Record *r = rec(key);
                if (!r) return;
                if (e.first.value(QLatin1String("ok")).toBool()) {
                    r->draft = e.first.value(QLatin1String("draft")).toObject();
                    r->description = e.second;
                    r->message.clear();
                } else {
                    r->message = e.first.value(QLatin1String("error")).toString();
                    r->isError = true;
                }
                bump();
                nextEdit(key);
            });
    });
}

void CsController::setName(const QString &key, const QString &name) {
    Record *r = rec(key);
    if (!r) return;
    const QString clean = HardwareController::sanitizeName(name);
    if (r->kind == QLatin1String("binding")) {
        r->nameEdit = clean;
        r->nameEdited = clean != nameOf(r->slot);
    } else if (r->kind == QLatin1String("group") || r->kind == QLatin1String("macro")) {
        r->draft.insert(QStringLiteral("name"), clean);
    }
    bump();
}

void CsController::revertRecord(const QString &key) {
    Record *r = rec(key);
    if (!r) return;
    r->draft = r->live;
    r->nameEdited = false;
    r->message.clear();
    // A receiver's Revert drops its remote buttons' edits too
    if (r->kind == QLatin1String("binding") && num(r->draft, "type") == Ir)
        for (int s = 0; s < irCount(); ++s)
            if (Record *ir = rec(keyOf(QStringLiteral("ir"), s))) {
                ir->draft = ir->live;
                ir->message.clear();
            }
    describeAll();
    bump();
}

void CsController::apply(const QString &key) {
    Record *r = rec(key);
    if (!r || r->applying) return;
    QList<QJsonObject> ops;
    QStringList irKeys;
    const QJsonValue index(r->slot);
    if (r->kind == QLatin1String("binding")) {
        if (r->draft != r->live)
            ops.append(QJsonObject{ { QStringLiteral("op"), QStringLiteral("binding") }, { QStringLiteral("index"), index },
                         { QStringLiteral("binding"), r->draft } });
        if (r->nameEdited)
            ops.append(QJsonObject{ { QStringLiteral("op"), QStringLiteral("name") }, { QStringLiteral("index"), index },
                         { QStringLiteral("name"), r->nameEdit } });
        // One Apply on the receiver sends its remote buttons' edits too
        if (num(r->draft, "type") == Ir)
            for (int s = 0; s < irCount(); ++s)
                if (irDirty(s)) {
                    const QString k = keyOf(QStringLiteral("ir"), s);
                    irKeys.append(k);
                    ops.append(QJsonObject{ { QStringLiteral("op"), QStringLiteral("ir") }, { QStringLiteral("index"), s },
                                 { QStringLiteral("command"), rec(k)->draft } });
                }
    } else if (r->kind == QLatin1String("group")) {
        ops.append(QJsonObject{ { QStringLiteral("op"), QStringLiteral("group") }, { QStringLiteral("index"), index },
                     { QStringLiteral("group"), r->draft } });
    } else if (r->kind == QLatin1String("macro")) {
        ops.append(QJsonObject{ { QStringLiteral("op"), QStringLiteral("macro") }, { QStringLiteral("index"), index },
                     { QStringLiteral("macro"), r->draft } });
    } else if (r->kind == QLatin1String("ir")) {
        ops.append(QJsonObject{ { QStringLiteral("op"), QStringLiteral("ir") }, { QStringLiteral("index"), index },
                     { QStringLiteral("command"), r->draft } });
    }
    if (ops.isEmpty()) return;
    r->applying = true;
    r->message.clear();
    for (const QString &k : irKeys) rec(k)->applying = true;
    bump();
    run(QStringLiteral("cs apply %1").arg(key), ops, [this, key, irKeys](const Reply &reply) {
        Record *r = rec(key);
        if (!r) return;
        r->applying = false;
        for (const QString &k : irKeys) rec(k)->applying = false;
        if (reply.ok) {
            r->draft = r->live;
            r->nameEdited = false;
            for (const QString &k : irKeys) {
                rec(k)->draft = rec(k)->live;
                rec(k)->message.clear();
            }
            r->message.clear();
        } else {
            // The refused draft stays, so what was wrong can be fixed and applied again
            r->message = reply.failure;
            r->isError = true;
        }
        describeAll();
        bump();
    });
}

void CsController::remove(const QString &key) {
    Record *r = rec(key);
    if (!r || r->applying) return;
    r->message.clear();
    r->nameEdited = false;
    QList<QJsonObject> ops;
    const QJsonValue index(r->slot);
    if (r->kind == QLatin1String("binding")) {
        if (num(r->live, "type") != None)
            ops.append(QJsonObject{ { QStringLiteral("op"), QStringLiteral("binding") }, { QStringLiteral("index"), index },
                         { QStringLiteral("binding"), QJsonObject() } });
        // A name set for the slot is device state too
        if (!nameOf(r->slot).isEmpty())
            ops.append(QJsonObject{ { QStringLiteral("op"), QStringLiteral("name") }, { QStringLiteral("index"), index },
                         { QStringLiteral("name"), QString() } });
    } else {
        const bool live = r->kind == QLatin1String("ir") ? num(r->live, "protocol") != 0
                        : r->kind == QLatin1String("group") ? num(r->live, "targetKind") != 0
                                                             : inUse(Record{ r->kind, r->slot, r->live, r->live });
        const char *field = r->kind == QLatin1String("ir") ? "command" : r->kind == QLatin1String("group") ? "group" : "macro";
        if (live)
            ops.append(QJsonObject{ { QStringLiteral("op"), r->kind }, { QStringLiteral("index"), index },
                         { QLatin1String(field), QJsonObject() } });
    }
    if (ops.isEmpty()) {
        // Never applied: dropping the draft is enough
        r->draft = r->live;
        describeAll();
        bump();
        return;
    }
    r->applying = true;
    bump();
    run(QStringLiteral("cs remove %1").arg(key), ops, [this, key](const Reply &reply) {
        Record *r = rec(key);
        if (!r) return;
        r->applying = false;
        r->draft = r->live;
        if (!reply.ok) {
            r->message = reply.failure;
            r->isError = true;
        }
        describeAll();
        bump();
    });
}

QString CsController::addControl(int type) {
    if (!writable()) return {};
    for (int s = 0; s < slotCount(); ++s) {
        const QString key = keyOf(QStringLiteral("binding"), s);
        Record *r = rec(key);
        if (!r || inUse(*r)) continue;
        // Claimed now, so a second add takes the next slot
        r->draft.insert(QStringLiteral("type"), type);
        r->editing = true;
        const int epoch = m_epoch;
        m_ctx.core->worker()->call(
            this,
            [s, type](FfiCore *c) {
                return parse(readSized([&](char *b, uint32_t n) { return dspi_cs_new_binding(c, uint32_t(s), uint8_t(type), b, n); }));
            },
            [this, key, epoch](const QJsonObject &made) {
                if (epoch != m_epoch) return;
                Record *r = rec(key);
                if (!r) return;
                r->editing = false;
                if (!made.value(QLatin1String("ok")).toBool()) {
                    r->draft = r->live;
                    bump();
                    return;
                }
                r->draft = made.value(QLatin1String("draft")).toObject();
                emit added(key);
                // A new control seeded with defaults works the moment it is added
                apply(key);
            });
        bump();
        return key;
    }
    return {};
}

QString CsController::addGroup() {
    for (int g = 0; g < groupCount(); ++g) {
        const QString key = keyOf(QStringLiteral("group"), g);
        Record *r = rec(key);
        if (!r || inUse(*r)) continue;
        // Not applied yet: a group with no members is not one the device stores
        r->draft = QJsonObject{ { QStringLiteral("targetKind"), kTargetOutputs }, { QStringLiteral("memberMask"), 0 },
                                { QStringLiteral("name"), tr("Group %1").arg(g + 1) } };
        r->message.clear();
        describeOne(key);
        bump();
        emit added(key);
        return key;
    }
    return {};
}

QString CsController::addMacro() {
    for (int m = 0; m < macroCount(); ++m) {
        const QString key = keyOf(QStringLiteral("macro"), m);
        Record *r = rec(key);
        if (!r || inUse(*r)) continue;
        r->draft = QJsonObject{ { QStringLiteral("name"), tr("Macro %1").arg(m + 1) }, { QStringLiteral("steps"), QJsonArray() } };
        r->message.clear();
        bump();
        emit added(key);
        return key;
    }
    return {};
}

void CsController::addRemoteButton() {
    for (int s = 0; s < irCount(); ++s) {
        const QString key = keyOf(QStringLiteral("ir"), s);
        Record *r = rec(key);
        if (!r || inUse(*r)) continue;
        r->editing = true;
        const int epoch = m_epoch;
        m_ctx.core->worker()->call(
            this,
            [](FfiCore *c) {
                const QJsonObject made = parse(readSized([c](char *b, uint32_t n) { return dspi_cs_new_record(c, "ir", b, n); }));
                const QJsonObject draft = made.value(QLatin1String("draft")).toObject();
                return std::make_pair(draft, draft.isEmpty() ? QJsonObject() : describeDraft(c, QStringLiteral("ir"), 0, draft));
            },
            [this, key, epoch](const std::pair<QJsonObject, QJsonObject> &made) {
                if (epoch != m_epoch) return;
                Record *r = rec(key);
                if (!r) return;
                r->editing = false;
                if (!made.first.isEmpty()) {
                    r->draft = made.first;
                    r->description = made.second;
                    r->message.clear();
                    emit added(key);
                }
                describeAll();
                bump();
            });
        return;
    }
}

// ---- macros -------------------------------------------------------------------------

void CsController::addStep(const QString &key) {
    Record *r = rec(key);
    if (!r || r->kind != QLatin1String("macro")) return;
    if (r->draft.value(QLatin1String("steps")).toArray().size() >= maxSteps()) return;
    queueEdit(key, [this, key]() {
        const int epoch = m_epoch;
        m_ctx.core->worker()->call(
            this,
            [](FfiCore *c) {
                return parse(readSized([c](char *b, uint32_t n) { return dspi_cs_new_record(c, "step", b, n); }))
                    .value(QLatin1String("draft"))
                    .toObject();
            },
            [this, key, epoch](const QJsonObject &step) {
                if (epoch != m_epoch) return;
                Record *r = rec(key);
                if (!r) return;
                QJsonArray steps = r->draft.value(QLatin1String("steps")).toArray();
                if (!step.isEmpty() && steps.size() < maxSteps()) steps.append(step);
                r->draft.insert(QStringLiteral("steps"), steps);
                describeOne(key);
                bump();
                nextEdit(key);
            });
    });
}

void CsController::moveStep(const QString &key, int step, int delta) {
    Record *r = rec(key);
    if (!r) return;
    QJsonArray steps = r->draft.value(QLatin1String("steps")).toArray();
    const int dest = step + delta;
    if (step < 0 || step >= steps.size() || dest < 0 || dest >= steps.size()) return;
    const QJsonValue a = steps.at(step);
    steps[step] = steps.at(dest);
    steps[dest] = a;
    r->draft.insert(QStringLiteral("steps"), steps);
    if (dest < r->stepDescriptions.size() && step < r->stepDescriptions.size()) {
        const QJsonValue d = r->stepDescriptions.at(step);
        r->stepDescriptions[step] = r->stepDescriptions.at(dest);
        r->stepDescriptions[dest] = d;
    }
    bump();
}

// The sequencer runs the steps in order, so a removed one closes the gap
void CsController::removeStep(const QString &key, int step) {
    Record *r = rec(key);
    if (!r) return;
    QJsonArray steps = r->draft.value(QLatin1String("steps")).toArray();
    if (step < 0 || step >= steps.size()) return;
    steps.removeAt(step);
    r->draft.insert(QStringLiteral("steps"), steps);
    if (step < r->stepDescriptions.size()) r->stepDescriptions.removeAt(step);
    bump();
}

void CsController::editStep(const QString &key, int step, const QString &row, const QVariant &value) {
    queueEdit(key, [this, key, step, row, value]() {
        Record *r = rec(key);
        if (!r) return;
        const QJsonArray steps = r->draft.value(QLatin1String("steps")).toArray();
        if (step < 0 || step >= steps.size()) {
            nextEdit(key);
            return;
        }
        const QJsonObject draft = steps.at(step).toObject();
        const int epoch = m_epoch;
        m_ctx.core->worker()->call(
            this,
            [draft, row, value](FfiCore *c) {
                const QJsonObject e = editDraft(c, QStringLiteral("step"), 0, draft, row, value);
                const QJsonObject next = e.value(QLatin1String("draft")).toObject();
                return std::make_pair(e, e.value(QLatin1String("ok")).toBool() ? describeDraft(c, QStringLiteral("step"), 0, next)
                                                                                : QJsonObject());
            },
            [this, key, step, epoch](const std::pair<QJsonObject, QJsonObject> &e) {
                if (epoch != m_epoch) return;
                Record *r = rec(key);
                if (!r) return;
                QJsonArray steps = r->draft.value(QLatin1String("steps")).toArray();
                if (e.first.value(QLatin1String("ok")).toBool() && step < steps.size()) {
                    steps[step] = e.first.value(QLatin1String("draft")).toObject();
                    r->draft.insert(QStringLiteral("steps"), steps);
                    while (r->stepDescriptions.size() <= step) r->stepDescriptions.append(QJsonObject());
                    r->stepDescriptions[step] = e.second;
                    r->message.clear();
                } else if (!e.first.value(QLatin1String("ok")).toBool()) {
                    r->message = e.first.value(QLatin1String("error")).toString();
                    r->isError = true;
                }
                bump();
                nextEdit(key);
            });
    });
}

void CsController::fire(const QString &key) {
    Record *r = rec(key);
    if (!r || r->kind != QLatin1String("macro")) return;
    const bool running = num(m_snap.value(QLatin1String("ext")).toObject(), "macroRunning") == r->slot;
    const QJsonObject op = running ? QJsonObject{ { QStringLiteral("op"), QStringLiteral("macroCancel") } }
                                   : QJsonObject{ { QStringLiteral("op"), QStringLiteral("macroFire") },
                                                  { QStringLiteral("index"), r->slot } };
    run(QStringLiteral("cs fire"), { op }, [this, key](const Reply &reply) {
        Record *r = rec(key);
        if (!r) return;
        if (!reply.ok) {
            r->message = reply.failure;
            r->isError = true;
            bump();
            return;
        }
        m_macroTicks = 0;
        m_ctx.poller->setEnabled(m_macroPoll, true);
    });
}

// ---- IR learn -----------------------------------------------------------------------

void CsController::startLearn(int sub) {
    const QString key = keyOf(QStringLiteral("ir"), sub);
    Record *r = rec(key);
    if (!r || m_learningSub >= 0) return;
    m_learningSub = sub;
    m_learnTicks = 0;
    m_learnMessage = tr("Point the remote at the receiver and press the button to learn.");
    r->message.clear();
    beginOp();
    run(QStringLiteral("cs learn"), { QJsonObject{ { QStringLiteral("op"), QStringLiteral("learnArm") } } },
        [this, key, sub](const Reply &reply) {
            if (m_learningSub != sub) return;
            if (reply.ok) {
                m_ctx.poller->setEnabled(m_learnPoll, true);
                bump();
                return;
            }
            endLearn();
            m_learnMessage.clear();
            if (Record *r = rec(key)) {
                r->message = reply.json.value(QLatin1String("status")).toInt() == kStatusNoIr
                                 ? tr("No IR receiver is active - apply the receiver first.")
                                 : reply.failure;
                r->isError = true;
            }
            bump();
        });
}

void CsController::finishLearn(const QJsonObject &result, bool timedOut) {
    const int sub = m_learningSub;
    endLearn();
    m_learnMessage.clear();
    Record *r = rec(keyOf(QStringLiteral("ir"), sub));
    if (!r) return;
    const int state = num(result, "state");
    const qint64 code = qint64(result.value(QLatin1String("code")).toDouble());
    if (timedOut || state != kLearnDone || code == 0) {
        const bool deviceTimeout = state == kLearnTimeout;
        if (!result.value(QLatin1String("ok")).toBool() && result.contains(QLatin1String("error"))) {
            r->message = result.value(QLatin1String("error")).toString();
            r->isError = true;
        } else {
            r->message = deviceTimeout ? tr("No remote button was detected - try again.") : tr("Learn stopped.");
            r->isError = deviceTimeout;
        }
        bump();
        return;
    }
    r->draft.insert(QStringLiteral("protocol"), num(result, "protocol"));
    r->draft.insert(QStringLiteral("code"), double(code));
    r->message = tr("Learned a %1 code. Apply to keep it.").arg(protocolName(num(result, "protocol")));
    r->isError = false;
    describeOne(keyOf(QStringLiteral("ir"), sub));
    bump();
}

void CsController::cancelLearn() {
    const int sub = m_learningSub;
    if (sub < 0) return;
    endLearn();
    m_learnMessage.clear();
    if (Record *r = rec(keyOf(QStringLiteral("ir"), sub))) {
        r->message = tr("Learn cancelled.");
        r->isError = false;
    }
    bump();
    run(QStringLiteral("cs learn cancel"), { QJsonObject{ { QStringLiteral("op"), QStringLiteral("learnCancel") } } }, {});
}

// Idempotent: a cancel, a device switch and the poll's end may race for it
void CsController::endLearn() {
    if (m_learningSub < 0) return;
    m_learningSub = -1;
    m_ctx.poller->setEnabled(m_learnPoll, false);
    endOp();
}

// ---- aux outputs --------------------------------------------------------------------

void CsController::setAuxState(int slot, bool on) {
    const QString key = keyOf(QStringLiteral("binding"), slot);
    run(QStringLiteral("cs aux state %1").arg(slot),
        { QJsonObject{ { QStringLiteral("op"), QStringLiteral("auxState") }, { QStringLiteral("index"), slot },
                       { QStringLiteral("on"), on ? 1 : 0 } } },
        [this, key](const Reply &reply) {
            if (Record *r = rec(key)) {
                r->message = reply.ok ? QString() : reply.failure;
                r->isError = !reply.ok;
                bump();
            }
        });
}

void CsController::setAuxLevel(int slot, double percent) {
    const QString key = keyOf(QStringLiteral("binding"), slot);
    // Latest wins: a slider's moves coalesce on the write's name
    run(QStringLiteral("cs aux level %1").arg(slot),
        { QJsonObject{ { QStringLiteral("op"), QStringLiteral("auxLevel") }, { QStringLiteral("index"), slot },
                       { QStringLiteral("percent"), std::clamp(percent, 0.0, 100.0) } } },
        [this, key](const Reply &reply) {
            if (Record *r = rec(key)) {
                r->message = reply.ok ? QString() : reply.failure;
                r->isError = !reply.ok;
                bump();
            }
        });
}

// ---- the display --------------------------------------------------------------------

QVariantMap CsController::display() const {
    const QJsonObject d = m_snap.value(QLatin1String("display")).toObject();
    const QJsonObject st = d.value(QLatin1String("status")).toObject();
    const QJsonArray pages = d.value(QLatin1String("pages")).toArray();
    const int maxPages = std::min(num(d, "maxPages"), int(pages.size()));
    QVariantList list;
    bool free = false;
    for (int i = 0; i < maxPages; ++i) {
        if (!(num(pages.at(i).toObject(), "flags") & kPageActive)) {
            free = true;
            continue;
        }
        const QJsonObject desc = m_pageDescriptions.value(i);
        list.append(QVariantMap{ { QStringLiteral("index"), i },
                                 { QStringLiteral("summary"), desc.value(QLatin1String("summary")).toString() },
                                 { QStringLiteral("rows"), rowsOf(desc, false) },
                                 { QStringLiteral("shown"), num(st, "page") == i } });
    }
    QString state;
    bool good = false;
    switch (num(st, "init")) {
    case 2: state = tr("Running"); good = true; break;
    case 1: state = tr("Starting up"); break;
    case 3: state = tr("Not responding"); break;
    default: state = tr("Not started"); break;
    }
    const int naks = num(st, "naks");
    return { { QStringLiteral("configured"), displaySlot() >= 0 },
             { QStringLiteral("rows"), rowsOf(m_displayDescription, false) },
             { QStringLiteral("pages"), list },
             { QStringLiteral("maxPages"), maxPages },
             { QStringLiteral("canAddPage"), free && writable() },
             { QStringLiteral("message"), m_displayMessage },
             { QStringLiteral("panelState"), state },
             { QStringLiteral("panelGood"), good },
             { QStringLiteral("panelError"), num(st, "init") == 3 },
             { QStringLiteral("panelDetail"),
               naks == 1 ? tr("1 I2C error so far. Check wiring, pull-up resistors, and the address.")
               : naks > 1 ? tr("%1 I2C errors so far. Check wiring, pull-up resistors, and the address.").arg(naks)
                          : tr("Reported by the device.") } };
}

void CsController::editDisplay(const QString &row, const QVariant &value) {
    const QJsonObject cfg = m_snap.value(QLatin1String("display")).toObject().value(QLatin1String("config")).toObject();
    const int epoch = m_epoch;
    m_ctx.core->worker()->call(
        this, [cfg, row, value](FfiCore *c) { return editDraft(c, QStringLiteral("display"), 0, cfg, row, value); },
        [this, epoch](const QJsonObject &e) {
            if (epoch != m_epoch) return;
            if (!e.value(QLatin1String("ok")).toBool()) {
                m_displayMessage = e.value(QLatin1String("error")).toString();
                bump();
                return;
            }
            run(QStringLiteral("cs display"),
                { QJsonObject{ { QStringLiteral("op"), QStringLiteral("displayConfig") },
                               { QStringLiteral("config"), e.value(QLatin1String("draft")).toObject() } } },
                [this](const Reply &reply) {
                    m_displayMessage = reply.ok ? QString() : reply.failure;
                    bump();
                });
        });
}

void CsController::editPage(int page, const QString &row, const QVariant &value) {
    const QJsonObject p = m_snap.value(QLatin1String("display")).toObject().value(QLatin1String("pages")).toArray().at(page).toObject();
    const int epoch = m_epoch;
    m_ctx.core->worker()->call(
        this, [p, page, row, value](FfiCore *c) { return editDraft(c, QStringLiteral("page"), page, p, row, value); },
        [this, page, epoch](const QJsonObject &e) {
            if (epoch != m_epoch) return;
            if (!e.value(QLatin1String("ok")).toBool()) {
                m_displayMessage = e.value(QLatin1String("error")).toString();
                bump();
                return;
            }
            run(QStringLiteral("cs display page %1").arg(page),
                { QJsonObject{ { QStringLiteral("op"), QStringLiteral("displayPage") }, { QStringLiteral("index"), page },
                               { QStringLiteral("page"), e.value(QLatin1String("draft")).toObject() } } },
                [this](const Reply &reply) {
                    m_displayMessage = reply.ok ? QString() : reply.failure;
                    bump();
                });
        });
}

void CsController::addPage() {
    const QJsonObject d = m_snap.value(QLatin1String("display")).toObject();
    const QJsonArray pages = d.value(QLatin1String("pages")).toArray();
    int index = -1;
    for (int i = 0; i < std::min(num(d, "maxPages"), int(pages.size())) && index < 0; ++i)
        if (!(num(pages.at(i).toObject(), "flags") & kPageActive)) index = i;
    if (index < 0) return;
    const int epoch = m_epoch;
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            return parse(readSized([c](char *b, uint32_t n) { return dspi_cs_new_record(c, "page", b, n); }))
                .value(QLatin1String("draft"))
                .toObject();
        },
        [this, index, epoch](const QJsonObject &page) {
            if (epoch != m_epoch || page.isEmpty()) return;
            run(QStringLiteral("cs display page %1").arg(index),
                { QJsonObject{ { QStringLiteral("op"), QStringLiteral("displayPage") }, { QStringLiteral("index"), index },
                               { QStringLiteral("page"), page } } },
                [this](const Reply &reply) {
                    m_displayMessage = reply.ok ? QString() : reply.failure;
                    bump();
                });
        });
}

void CsController::removePage(int page) {
    run(QStringLiteral("cs display page %1").arg(page),
        { QJsonObject{ { QStringLiteral("op"), QStringLiteral("displayPage") }, { QStringLiteral("index"), page },
                       { QStringLiteral("page"), QJsonObject() } } },
        [this](const Reply &reply) {
            m_displayMessage = reply.ok ? QString() : reply.failure;
            bump();
        });
}

void CsController::watchDisplay(bool on) {
    m_watchDisplay = on;
    m_ctx.poller->setEnabled(m_displayPoll, on && displaySlot() >= 0);
}

// ---- the save bar -------------------------------------------------------------------

void CsController::save(std::function<void(const QStringList &failures)> done) {
    run(QStringLiteral("cs save"), { QJsonObject{ { QStringLiteral("op"), QStringLiteral("save") } } },
        [done](const Reply &reply) {
            if (!done) return;
            if (reply.ok) done({});
            else done({ tr("the control surfaces (%1)").arg(reply.failure) });
        });
}

// The device puts back what is stored; every draft then follows it
void CsController::revert() {
    if (m_learningSub >= 0) cancelLearn();
    run(QStringLiteral("cs revert"), { QJsonObject{ { QStringLiteral("op"), QStringLiteral("revert") } } },
        [this](const Reply &reply) {
            for (Record &r : m_records) {
                r.draft = r.live;
                r.nameEdited = false;
                r.message.clear();
            }
            if (!reply.ok) m_loadError = reply.failure;
            describeAll();
            bump();
        });
}
