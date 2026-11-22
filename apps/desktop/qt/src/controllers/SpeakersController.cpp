// -- WP ROLEQ -- The speaker layout: the Getting Started step, Settings ›
// Speakers and the Measure tables' write-back (SpeakersController.h)
#include "SpeakersController.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "FilesController.h"
#include "SessionState.h"
#include "SettingsController.h"
#include "Snapshot.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QMetaObject>
#include <QQmlEngine>
#include <QSettings>
#include <algorithm>

namespace {

const Registrar::Controller<SpeakersController> speakersController;
const SettingsRegistry::Page speakersPage{
    "speakers", SettingsRegistry::System, 55, QT_TRANSLATE_NOOP("SettingsPages", "Speakers"), "volume-2", "SpeakersPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Speaker layout"), QT_TRANSLATE_NOOP("SettingsPages", "Roles"),
      QT_TRANSLATE_NOOP("SettingsPages", "Tweeter"), QT_TRANSLATE_NOOP("SettingsPages", "Woofer"),
      QT_TRANSLATE_NOOP("SettingsPages", "Subwoofer"), QT_TRANSLATE_NOOP("SettingsPages", "Way"),
      QT_TRANSLATE_NOOP("SettingsPages", "Confirm") }
};

// A layout's speaker name, characters at most (the core's bound)
constexpr int kMaxSpeakerChars = 64;
// A layout's reply: 16 outputs with their names fit many times over; a set is
// never called twice to fit its answer
constexpr int kLayoutBytes = 64 * 1024;

QString str(const QJsonObject &o, const char *key) { return o.value(QLatin1String(key)).toString(); }

bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }

QJsonObject object(const QByteArray &text) { return QJsonDocument::fromJson(text).object(); }

// A reply's layout: the object without `ok`, {} for `value: null` or a failure
QJsonObject layoutOf(const QJsonObject &reply) {
    if (!isOk(reply) || !reply.value(QLatin1String("outputs")).isArray()) return {};
    QJsonObject l = reply;
    l.remove(QStringLiteral("ok"));
    return l;
}

qint64 stampOf(const QJsonObject &layout) { return layout.value(QLatin1String("updatedMs")).toVariant().toLongLong(); }

// What this console wrote last for a device (its updatedMs)
QString writtenKey(const QString &serial) {
    return QStringLiteral("speakers/written/") + QString::fromLatin1(serial.toUtf8().toHex());
}

QJsonObject rowIn(const QJsonObject &layout, int output) {
    for (const QJsonValue &v : layout.value(QLatin1String("outputs")).toArray())
        if (v.toObject().value(QLatin1String("output")).toInt() == output) return v.toObject();
    return {};
}

// `layout` with output `o`'s row patched (added when it has none), in output order
QJsonObject withRow(QJsonObject layout, int o, const QString &role, const QString &speaker, int group) {
    QJsonArray outs;
    QJsonObject row{ { QStringLiteral("output"), o }, { QStringLiteral("role"), QStringLiteral("full") },
                     { QStringLiteral("speaker"), QString() }, { QStringLiteral("group"), 0 } };
    for (const QJsonValue &v : layout.value(QLatin1String("outputs")).toArray()) {
        if (v.toObject().value(QLatin1String("output")).toInt() == o) row = v.toObject();
        else outs.append(v);
    }
    if (!role.isEmpty()) row.insert(QStringLiteral("role"), role);
    if (!speaker.isEmpty()) row.insert(QStringLiteral("speaker"), speaker.left(kMaxSpeakerChars));
    if (group >= 0) row.insert(QStringLiteral("group"), std::clamp(group, 0, 15));
    outs.append(row);
    QList<QJsonValue> sorted(outs.begin(), outs.end());
    std::sort(sorted.begin(), sorted.end(), [](const QJsonValue &a, const QJsonValue &b) {
        return a.toObject().value(QLatin1String("output")).toInt() < b.toObject().value(QLatin1String("output")).toInt();
    });
    QJsonArray out;
    for (const QJsonValue &v : sorted) out.append(v);
    layout.insert(QStringLiteral("outputs"), out);
    return layout;
}

QByteArray setLayout(FfiCore *c, const QJsonObject &layout) {
    const QByteArray text = QJsonDocument(layout).toJson(QJsonDocument::Compact);
    QByteArray buf(kLayoutBytes, '\0');
    const uint32_t n = dspi_speaker_layout_set(c, text.constData(), buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

QByteArray readLayout(FfiCore *c, uint32_t (*fill)(FfiCore *, char *, uint32_t)) {
    return readSized([c, fill](char *b, uint32_t l) { return fill(c, b, l); }, 4096);
}

} // namespace

SpeakersController::SpeakersController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &SpeakersController::onSnapshot);
    // The other controllers exist once every constructor has run: an imported
    // configuration may bring a layout (unconfirmed)
    QMetaObject::invokeMethod(
        this,
        [this] {
            if (auto *files = Registrar::instance<FilesController>())
                connect(files, &FilesController::importingChanged, this, [this, files] {
                    if (!files->importing()) reload();
                });
        },
        Qt::QueuedConnection);
}

SpeakersController *SpeakersController::create(QQmlEngine *, QJSEngine *) {
    SpeakersController *self = Registrar::instance<SpeakersController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

QVariantList SpeakersController::outputs() const {
    QVariantList outs;
    ChannelListModel *ch = m_ctx.device->channels();
    for (int r = 0; r < ch->rowCount(); ++r) {
        if (ch->get(r, QStringLiteral("kind")).toString() != QLatin1String("output")) continue;
        outs.append(QVariantMap{ { QStringLiteral("output"), ch->get(r, QStringLiteral("number")) },
                                 { QStringLiteral("label"), ch->get(r, QStringLiteral("label")) } });
    }
    return outs;
}

SpeakersController::StatePtr SpeakersController::state() const {
    return sessionState<State>(m_ctx.activeSession(), QStringLiteral("SpeakersController"));
}

void SpeakersController::onSnapshot() {
    const StatePtr st = state();
    const bool editable = m_ctx.device->connected() && !m_ctx.device->readOnly();
    if (editable != m_editable) {
        m_editable = editable;
        emit editableChanged();
    }
    // -- WP MDQT2c -- another device is active: its layout and its draft, as it
    // was left; what others stored for it meanwhile is read
    const bool switched = st != m_announced;
    m_announced = st;
    const QString serial = m_ctx.device->connected() ? m_ctx.device->serial() : QString();
    const QVariantList outs = outputs();
    if (serial != st->serial) {
        st->serial = serial;
        st->outputs = outs;
        ++st->generation;
        st->loaded = false;
        st->stored = {};
        st->proposal = {};
        st->notes.clear();
        st->rows.clear();
        st->touched = false;
        emit storedChanged(QStringLiteral("read"));
        emit rowsChanged();
        if (switched) emit saidChanged();
        if (!serial.isEmpty()) reload();
        return;
    }
    if (outs != st->outputs) {
        // The labels follow the device; the draft keeps its choices
        st->outputs = outs;
        QVariantList rows;
        for (const QVariant &o : outs) {
            QVariantMap row = rowOf(*st, o.toMap().value(QStringLiteral("output")).toInt());
            for (const QVariant &old : std::as_const(st->rows))
                if (old.toMap().value(QStringLiteral("output")) == o.toMap().value(QStringLiteral("output"))) row = old.toMap();
            row.insert(QStringLiteral("label"), o.toMap().value(QStringLiteral("label")));
            rows.append(row);
        }
        st->rows = rows;
        if (!switched) emit rowsChanged();
    }
    if (!switched) return;
    emit storedChanged(QStringLiteral("read"));
    emit rowsChanged();
    emit saidChanged();
    reload();
}

void SpeakersController::remember(const QString &serial, const QJsonObject &kept) {
    QSettings().setValue(writtenKey(serial), stampOf(kept));
}

// An unconfirmed layout this console did not write came with an import
bool SpeakersController::imported(const QString &serial, const QJsonObject &layout) {
    if (layout.isEmpty() || layout.value(QLatin1String("confirmedByUser")).toBool()) return false;
    return QSettings().value(writtenKey(serial), qint64(-1)).toLongLong() != stampOf(layout);
}

QString SpeakersController::status() const {
    const StatePtr st = state();
    if (st->serial.isEmpty()) return QStringLiteral("disconnected");
    if (st->stored.isEmpty()) return QStringLiteral("none");
    if (st->stored.value(QLatin1String("confirmedByUser")).toBool()) return QStringLiteral("confirmed");
    return imported(st->serial, st->stored) ? QStringLiteral("imported") : QStringLiteral("unconfirmed");
}

qint64 SpeakersController::stamp() const { return stampOf(state()->stored); }

bool SpeakersController::editable() const { return m_editable; }

void SpeakersController::reload() {
    const StatePtr st = state();
    if (st->serial.isEmpty()) return;
    const quint64 generation = st->generation;
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            return std::make_pair(readLayout(c, dspi_speaker_layout), readLayout(c, dspi_speaker_layout_propose));
        },
        [this, st, generation](const std::pair<QByteArray, QByteArray> &r) {
            if (generation != st->generation) return;
            const QJsonObject stored = object(r.first), proposal = object(r.second);
            if (!isOk(stored)) say(st, str(stored, "error"), true);
            st->proposal = layoutOf(proposal);
            st->notes.clear();
            for (const QJsonValue &n : st->proposal.value(QLatin1String("notes")).toArray()) st->notes << n.toString();
            const QJsonObject layout = layoutOf(stored);
            if (!st->loaded) {
                st->loaded = true;
                st->stored = layout;
                rebuild(st);
                if (shown(st)) emit storedChanged(QStringLiteral("read"));
            } else if (layout != st->stored) {
                setStored(st, layout, QStringLiteral("read"));
            }
        });
}

void SpeakersController::setStored(const StatePtr &st, const QJsonObject &layout, const QString &origin) {
    // The person's unconfirmed edits stay in the draft
    const bool keepDraft = st->touched && origin != QLatin1String("confirm");
    st->stored = layout;
    if (!keepDraft) rebuild(st);
    if (shown(st)) emit storedChanged(origin);
}

void SpeakersController::rebuild(const StatePtr &st) {
    QVariantList rows;
    for (const QVariant &o : std::as_const(st->outputs)) {
        QVariantMap row = rowOf(*st, o.toMap().value(QStringLiteral("output")).toInt());
        row.insert(QStringLiteral("label"), o.toMap().value(QStringLiteral("label")));
        rows.append(row);
    }
    st->rows = rows;
    st->touched = false;
    if (shown(st)) emit rowsChanged();
}

QVariantMap SpeakersController::rowOf(const State &st, int output) {
    const QJsonObject base = st.stored.isEmpty() ? st.proposal : st.stored;
    const QJsonObject r = rowIn(base, output);
    return QVariantMap{ { QStringLiteral("output"), output },
                        { QStringLiteral("included"), !r.isEmpty() },
                        { QStringLiteral("role"), r.isEmpty() ? QStringLiteral("full") : str(r, "role") },
                        { QStringLiteral("speaker"), str(r, "speaker") },
                        { QStringLiteral("group"), r.value(QLatin1String("group")).toInt() } };
}

QVariantMap SpeakersController::rowOf(int output) const { return rowOf(*state(), output); }

QString SpeakersController::roleOf(int output) const {
    const StatePtr st = state();
    const QJsonObject r = rowIn(st->stored.isEmpty() ? st->proposal : st->stored, output);
    return str(r, "role");
}

void SpeakersController::setField(int output, const QString &field, const QVariant &value) {
    const StatePtr st = state();
    for (QVariant &v : st->rows) {
        QVariantMap m = v.toMap();
        if (m.value(QStringLiteral("output")).toInt() != output) continue;
        if (m.value(field) == value) return;
        m.insert(field, value);
        v = m;
        st->touched = true;
        emit rowsChanged();
        return;
    }
}

void SpeakersController::setIncluded(int output, bool on) { setField(output, QStringLiteral("included"), on); }
void SpeakersController::setRole(int output, const QString &role) { setField(output, QStringLiteral("role"), role); }
void SpeakersController::setSpeaker(int output, const QString &speaker) {
    setField(output, QStringLiteral("speaker"), speaker.trimmed().left(kMaxSpeakerChars));
}
void SpeakersController::setGroup(int output, int group) { setField(output, QStringLiteral("group"), std::clamp(group, 0, 15)); }

QJsonObject SpeakersController::draftLayout() const {
    QJsonArray outs;
    for (const QVariant &v : std::as_const(state()->rows)) {
        const QVariantMap m = v.toMap();
        if (!m.value(QStringLiteral("included")).toBool()) continue;
        outs.append(QJsonObject{ { QStringLiteral("output"), m.value(QStringLiteral("output")).toInt() },
                                 { QStringLiteral("role"), m.value(QStringLiteral("role")).toString() },
                                 { QStringLiteral("speaker"), m.value(QStringLiteral("speaker")).toString() },
                                 { QStringLiteral("group"), m.value(QStringLiteral("group")).toInt() } });
    }
    return QJsonObject{ { QStringLiteral("outputs"), outs } };
}

bool SpeakersController::edited() const {
    const StatePtr st = state();
    if (st->stored.isEmpty()) return true;
    return draftLayout().value(QLatin1String("outputs")) != st->stored.value(QLatin1String("outputs"));
}

void SpeakersController::useSuggestion() {
    const StatePtr st = state();
    if (st->proposal.isEmpty()) return;
    QVariantList rows;
    for (const QVariant &o : std::as_const(st->outputs)) {
        const int n = o.toMap().value(QStringLiteral("output")).toInt();
        const QJsonObject r = rowIn(st->proposal, n);
        rows.append(QVariantMap{ { QStringLiteral("output"), n },
                                 { QStringLiteral("label"), o.toMap().value(QStringLiteral("label")) },
                                 { QStringLiteral("included"), !r.isEmpty() },
                                 { QStringLiteral("role"), r.isEmpty() ? QStringLiteral("full") : str(r, "role") },
                                 { QStringLiteral("speaker"), str(r, "speaker") },
                                 { QStringLiteral("group"), r.value(QLatin1String("group")).toInt() } });
    }
    st->rows = rows;
    st->touched = true;
    emit rowsChanged();
}

void SpeakersController::confirm() {
    const StatePtr st = state();
    if (!m_editable || st->serial.isEmpty()) return;
    QJsonObject layout = draftLayout();
    if (layout.value(QLatin1String("outputs")).toArray().isEmpty()) {
        say(st, tr("Choose at least one output for the speaker layout."), true);
        return;
    }
    layout.insert(QStringLiteral("confirmedByUser"), true);
    const quint64 generation = st->generation;
    m_ctx.core->worker()->call(
        this, [layout](FfiCore *c) { return setLayout(c, layout); },
        [this, st, generation, serial = st->serial](const QByteArray &r) {
            // Another device on this session by now: kept for its own all the same
            if (generation != st->generation) {
                if (const QJsonObject o = object(r); isOk(o)) remember(serial, layoutOf(o));
                return;
            }
            const QJsonObject o = object(r);
            if (!isOk(o)) {
                say(st, tr("Not confirmed: %1").arg(r.isEmpty() ? tr("the core did not answer.") : str(o, "error")), true);
                return;
            }
            remember(serial, layoutOf(o));
            setStored(st, layoutOf(o), QStringLiteral("confirm"));
            say(st, tr("Confirmed: the measurement takes its roles from this layout."), false);
        });
}

void SpeakersController::writeBack(int output, const QString &role, const QString &speaker, int group) {
    const StatePtr st = state();
    if (!m_editable || st->serial.isEmpty()) return;
    const quint64 generation = st->generation;
    const QString serial = st->serial;
    // Read, change and keep in one job: two quick edits never undo each other
    m_ctx.core->worker()->call(
        this,
        [output, role, speaker, group](FfiCore *c) {
            const QJsonObject stored = layoutOf(object(readLayout(c, dspi_speaker_layout)));
            QJsonObject base = stored.isEmpty() ? layoutOf(object(readLayout(c, dspi_speaker_layout_propose))) : stored;
            // The confirmation stays as it was: only Confirm confirms
            base.insert(QStringLiteral("confirmedByUser"), stored.value(QLatin1String("confirmedByUser")).toBool());
            base.insert(QStringLiteral("notes"), stored.value(QLatin1String("notes")).toArray());
            return std::make_pair(stored, setLayout(c, withRow(base, output, role, speaker, group)));
        },
        [this, st, generation, serial](const std::pair<QJsonObject, QByteArray> &p) {
            const QByteArray &r = p.second;
            const QJsonObject o = object(r);
            // -- WP INT2 -- The layouts are files now (MDQT1's folder): what was
            // kept is remembered also when another device is active by now, and
            // judged by the layout it replaced, not by the one last read here.
            // An imported layout stays imported until Confirm
            if (isOk(o) && !imported(serial, p.first)) remember(serial, layoutOf(o));
            if (generation != st->generation) return;
            if (!isOk(o)) {
                say(st, tr("Not saved to your speaker layout: %1").arg(r.isEmpty() ? tr("the core did not answer.") : str(o, "error")),
                    true, true);
                return;
            }
            setStored(st, layoutOf(o), QStringLiteral("measure"));
            say(st, tr("Saved to your speaker layout."), false, true);
        });
}

QString SpeakersController::bannerText(bool here) const {
    const QString s = status();
    // -- WP MCP5 -- (review RB C4) "imported" is any unconfirmed layout this console
    // did not write: a configuration file or another app (never an agent: no tool
    // writes it), so the words do not claim a file
    if (s == QLatin1String("imported"))
        return here ? tr("Imported speaker layout: these roles were set outside this app (a configuration file or "
                         "another app). Check that they match this DSPi's speakers, then confirm them.")
                    : tr("Imported speaker layout: these roles were set outside this app (a configuration file or "
                         "another app). Check that they match this DSPi's speakers, then confirm them in Settings › "
                         "Speakers.");
    if (s == QLatin1String("unconfirmed"))
        return here ? tr("Not confirmed yet: check which output drives what, then confirm.")
                    : tr("Your speaker layout is not confirmed yet. Check which output drives what, then confirm it in "
                         "Settings › Speakers.");
    if (s == QLatin1String("none") && state()->loaded)
        return here ? tr("Suggested from the filters and the routing: check which output drives what, then confirm.")
                    : tr("No speaker layout yet: the roles are suggested from the filters. Confirm which output drives "
                         "what in Settings › Speakers.");
    return {};
}

void SpeakersController::openSettings() {
    if (auto *settings = Registrar::instance<SettingsController>()) settings->showPage(QStringLiteral("speakers"));
}

void SpeakersController::say(const StatePtr &st, const QString &text, bool error, bool measure) {
    st->said = text;
    st->saidError = error;
    st->saidInMeasure = measure;
    if (shown(st)) emit saidChanged();
}
