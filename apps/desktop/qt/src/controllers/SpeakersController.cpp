// -- WP ROLEQ -- The speaker layout: the Getting Started step, Settings ›
// Speakers and the Measure tables' write-back (SpeakersController.h)
#include "SpeakersController.h"

#include "AiController.h"        // -- WP ADETQ --
#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceSession.h"       // -- WP ADETQ --
#include "FilesController.h"
#include "MeasureController.h"   // -- WP ADETQ --
#include "NoticeCenter.h"        // -- WP ADETQ --
#include "SessionState.h"
#include "SettingsController.h"
#include "Snapshot.h"

#include <QDateTime>   // -- WP ADETQ --
#include <QJsonArray>
#include <QJsonDocument>
#include <QLocale>     // -- WP ADETQ --
#include <QMetaObject>
#include <QQmlEngine>
#include <QSet>        // -- WP UB7 --
#include <QSettings>
#include <algorithm>

namespace {

const Registrar::Controller<SpeakersController> speakersController;
// -- WP ADETQ -- a kept layout that is not confirmed, on the main screen (both layouts)
const Registrar::Banner speakersLayoutNote("SpeakersLayoutNote", 60, Registrar::Blocking);
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
                     { QStringLiteral("speaker"), QString() }, { QStringLiteral("group"), 0 },
                     { QStringLiteral("source"), QStringLiteral("user") } };   // -- WP ADETQ --
    for (const QJsonValue &v : layout.value(QLatin1String("outputs")).toArray()) {
        if (v.toObject().value(QLatin1String("output")).toInt() == o) row = v.toObject();
        else outs.append(v);
    }
    // -- WP ADETQ -- a role the person gives is theirs (a speaker or group edit keeps the source)
    if (!role.isEmpty() && row.value(QLatin1String("role")).toString() != role) row.insert(QStringLiteral("source"), QStringLiteral("user"));
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

// -- WP ADETQ -- SpeakerLayout::state(): "confirmed", "autoDetected" (a detection
// recorded and no row a mere suggestion), "unconfirmed"; "" for none
// -- WP ROLES -- or "stated": set by an AI agent from the user's words (a
// statement recorded, a stated row, no suggestion)
QString stateOf(const QJsonObject &layout) {
    if (layout.isEmpty()) return {};
    if (layout.value(QLatin1String("confirmedByUser")).toBool()) return QStringLiteral("confirmed");
    const QJsonArray outs = layout.value(QLatin1String("outputs")).toArray();
    bool anyone = !outs.isEmpty(), stated = false;
    for (const QJsonValue &v : outs) {
        const QString source = v.toObject().value(QLatin1String("source")).toString();
        anyone = anyone && source != QLatin1String("suggested") && !source.isEmpty();
        stated = stated || source == QLatin1String("stated");
    }
    if (!anyone) return QStringLiteral("unconfirmed");
    if (stated && layout.value(QLatin1String("stated")).isObject()) return QStringLiteral("stated");
    return layout.value(QLatin1String("detected")).isObject() ? QStringLiteral("autoDetected") : QStringLiteral("unconfirmed");
}

// -- WP ROLES -- the top-fall check of `output`'s stated role, {} for none
QJsonObject checkIn(const QJsonObject &layout, int output) {
    const QJsonObject row = rowIn(layout, output);
    for (const QJsonValue &v : layout.value(QLatin1String("stated")).toObject().value(QLatin1String("checks")).toArray())
        if (v.toObject().value(QLatin1String("output")).toInt() == output && str(v.toObject(), "role") == str(row, "role"))
            return v.toObject();
    return {};
}

// -- WP ADETQ -- the detection's record of `output`, {} for none
QJsonObject detectedIn(const QJsonObject &layout, int output) {
    for (const QJsonValue &v : layout.value(QLatin1String("detected")).toObject().value(QLatin1String("outputs")).toArray())
        if (v.toObject().value(QLatin1String("output")).toInt() == output) return v.toObject();
    return {};
}

// -- WP UB7 -- a speaker's name as the groups compare it ("" for none)
QString nameKey(const QVariantMap &row) { return row.value(QStringLiteral("speaker")).toString().trimmed().toCaseFolded(); }
bool inLayout(const QVariantMap &row) { return row.value(QStringLiteral("included")).toBool(); }
int groupOf(const QVariantMap &row) { return row.value(QStringLiteral("group")).toInt(); }

// -- WP UB7 -- the lowest group none of `used` has
int freeGroup(const QSet<int> &used) {
    int g = 0;
    while (used.contains(g) && g < 15) ++g;
    return g;
}

// -- WP ADETQ -- "today at 14:02", or "on 23/11/2026"
QString stampAt(qint64 ms) {
    const QDateTime at = QDateTime::fromMSecsSinceEpoch(ms);
    if (at.date() == QDate::currentDate())
        //: When a speaker layout was kept: %1 a time of day
        return SpeakersController::tr("today at %1").arg(QLocale().toString(at.time(), QLocale::ShortFormat));
    //: When a speaker layout was kept: %1 a date
    return SpeakersController::tr("on %1").arg(QLocale().toString(at.date(), QLocale::ShortFormat));
}

// -- WP ADETQ -- "500 Hz", "2.5 kHz", "—" for none
QString hzWords(const QJsonValue &v) {
    if (!v.isDouble() || v.toDouble() <= 0) return QStringLiteral("—");
    const double hz = v.toDouble();
    return hz < 1000 ? SpeakersController::tr("%1 Hz").arg(qRound(hz))
                     : SpeakersController::tr("%1 kHz").arg(QLocale().toString(hz / 1000.0, 'g', 3));
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
            // -- WP ADETQ -- whether a detection may start follows what runs
            if (auto *measure = Registrar::instance<MeasureController>()) {
                connect(measure, &MeasureController::runningChanged, this, [this] {
                    const bool now = detecting();
                    if (m_wasDetecting && !now) m_detectAsked = false;
                    m_wasDetecting = now;
                    emit detectChanged();
                });
                connect(measure, &MeasureController::deviceChanged, this, &SpeakersController::detectChanged);
                // Why the page's detection did not start, or stopped: said under its buttons
                connect(measure, &MeasureController::errorChanged, this, [this, measure] {
                    if (m_detectAsked && !measure->error().isEmpty()) say(state(), measure->error(), true);
                });
            }
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
    // -- WP ADETQ -- the detection wrote it: not an import
    if (stateOf(st->stored) == QLatin1String("autoDetected")) return QStringLiteral("autoDetected");
    // -- WP ROLES -- an agent set it from the person's words: not an import either
    if (stateOf(st->stored) == QLatin1String("stated")) return QStringLiteral("stated");
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
    // -- INTC (DL7 fold) -- a layout written elsewhere (an agent's roles from
    // the person's words, a detection, another console) makes what this page
    // said of its own Confirm or save stale: no "Confirmed: …" under roles an
    // agent set. An error stays until the next action.
    if (origin == QLatin1String("read") && !st->saidError && !st->said.isEmpty()) say(st, QString(), false);
    if (!keepDraft) rebuild(st);
    else if (shown(st)) emit rowsChanged();   // -- WP ADETQ -- the pills are judged against it
    if (shown(st)) emit storedChanged(origin);
    if (shown(st)) emit detectChanged();      // -- WP ADETQ -- a confirmed layout is not detected over
    // -- WP SHOWFIX -- the channels' names follow the layout ("Left tweeter"):
    // the snapshot carries the core's channel words, and no state changed
    if (shown(st) && m_ctx.device) m_ctx.device->reread();
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
                        { QStringLiteral("group"), r.value(QLatin1String("group")).toInt() },
                        { QStringLiteral("source"), str(r, "source") } };   // -- WP ADETQ --
}

QVariantMap SpeakersController::rowOf(int output) const { return rowOf(*state(), output); }

// -- WP SHOWFIX -- (finding 8)
QVariantMap SpeakersController::pillFor(int output) const {
    const StatePtr st = state();
    return pillOf(*st, rowOf(*st, output));
}

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
        // -- WP ADETQ -- a role (or an output taken in) the person gives is theirs
        if (field == QLatin1String("role") || (field == QLatin1String("included") && value.toBool()))
            m.insert(QStringLiteral("source"), QStringLiteral("user"));
        v = m;
        st->touched = true;
        emit rowsChanged();
        return;
    }
}

void SpeakersController::setIncluded(int output, bool on) {
    setField(output, QStringLiteral("included"), on);
    if (on) settleGroup(output);   // -- WP UB7 --
}
void SpeakersController::setRole(int output, const QString &role) { setField(output, QStringLiteral("role"), role); }
void SpeakersController::setSpeaker(int output, const QString &speaker) {
    setField(output, QStringLiteral("speaker"), speaker.trimmed().left(kMaxSpeakerChars));
    settleGroup(output);   // -- WP UB7 --
}

// -- WP UB7 -- (UXP B7) The row's group as its speaker's name says: the group of
// another row with the name, else its own (a new one when the group it has is
// another speaker's). An unnamed row keeps its group.
void SpeakersController::settleGroup(int output) {
    const StatePtr st = state();
    QVariantMap row;
    for (const QVariant &v : std::as_const(st->rows))
        if (v.toMap().value(QStringLiteral("output")).toInt() == output) row = v.toMap();
    const QString key = nameKey(row);
    if (!inLayout(row) || key.isEmpty()) return;
    const int was = groupOf(row);
    QSet<int> used;
    int same = -1;
    bool shared = false;
    for (const QVariant &v : std::as_const(st->rows)) {
        const QVariantMap r = v.toMap();
        if (!inLayout(r) || r.value(QStringLiteral("output")).toInt() == output) continue;
        used.insert(groupOf(r));
        if (same < 0 && nameKey(r) == key) same = groupOf(r);
        shared = shared || groupOf(r) == was;
    }
    const int group = same >= 0 ? same : shared ? freeGroup(used) : was;
    if (group != was) setField(output, QStringLiteral("group"), group);
}

// -- WP UB7 -- every row's group as the names say, in output order: a name keeps
// the group its first row has unless an earlier name took it
void SpeakersController::followNames() {
    const StatePtr st = state();
    QHash<QString, int> groups;
    QSet<int> used;
    QVariantList rows = st->rows;
    for (const bool named : { true, false })
        for (QVariant &v : rows) {
            QVariantMap r = v.toMap();
            const QString key = nameKey(r);
            if (!inLayout(r) || key.isEmpty() != !named) continue;
            int g = groups.value(key, -1);
            if (g < 0) {
                g = used.contains(groupOf(r)) ? freeGroup(used) : groupOf(r);
                used.insert(g);
                if (named) groups.insert(key, g);
            }
            r.insert(QStringLiteral("group"), g);
            v = r;
        }
    if (rows == st->rows) return;
    st->rows = rows;
    st->touched = true;
    emit rowsChanged();
}

bool SpeakersController::groupsFollowNames() const {
    const QVariantList &rows = state()->rows;
    for (int i = 0; i < rows.size(); ++i)
        for (int j = i + 1; j < rows.size(); ++j) {
            const QVariantMap a = rows[i].toMap(), b = rows[j].toMap();
            if (!inLayout(a) || !inLayout(b)) continue;
            const QString ka = nameKey(a), kb = nameKey(b);
            if (ka.isEmpty() && kb.isEmpty()) continue;
            if ((ka == kb) != (groupOf(a) == groupOf(b))) return false;
        }
    return true;
}

namespace {
// -- WP UB7 -- a role in the summary's words (the glossary's: woofer, mid driver, …)
QString roleWord(const QString &role) {
    if (role == QLatin1String("sub")) return SpeakersController::tr("subwoofer");
    if (role == QLatin1String("low")) return SpeakersController::tr("woofer");
    if (role == QLatin1String("mid")) return SpeakersController::tr("mid driver");
    if (role == QLatin1String("high")) return SpeakersController::tr("tweeter");
    return SpeakersController::tr("full range");
}
} // namespace

QString SpeakersController::summary() const {
    const StatePtr st = state();
    if (st->serial.isEmpty() || !st->loaded) return {};
    const QJsonObject base = st->stored.isEmpty() ? st->proposal : st->stored;
    QStringList order;
    QHash<QString, QStringList> parts;
    QHash<QString, QString> names;
    for (const QJsonValue &v : base.value(QLatin1String("outputs")).toArray()) {
        const QJsonObject o = v.toObject();
        const int n = o.value(QLatin1String("output")).toInt();
        QString label = tr("Out %1").arg(n + 1);
        for (const QVariant &x : std::as_const(st->outputs))
            if (x.toMap().value(QStringLiteral("output")).toInt() == n) label = x.toMap().value(QStringLiteral("label")).toString();
        const QString speaker = str(o, "speaker").trimmed();
        // An unnamed output stands alone
        const QString key = speaker.isEmpty() ? QStringLiteral("\x01") + QString::number(n) : speaker.toCaseFolded();
        if (!parts.contains(key)) {
            order << key;
            names.insert(key, speaker);
        }
        //: The speaker summary's part for one output: %1 its role ("tweeter"), %2 the output ("Out 2")
        parts[key] << tr("%1 %2").arg(roleWord(str(o, "role")), label);
    }
    if (order.isEmpty()) return tr("No output is in the speaker layout yet.");
    QStringList speakers;
    for (const QString &key : std::as_const(order))
        speakers << (names.value(key).isEmpty() ? parts.value(key).join(QStringLiteral(", "))
                                                //: The speaker summary's part for one speaker: %1 its name ("L"), %2 its outputs ("woofer Out 1, tweeter Out 2")
                                                : tr("%1: %2").arg(names.value(key), parts.value(key).join(QStringLiteral(", "))));
    const QString s = status();
    //: The end of the speaker summary: the layout is confirmed
    const QString said = s == QLatin1String("confirmed") ? tr("confirmed")
                          //: The end of the speaker summary: nothing is kept, the roles are the suggestion
                          : s == QLatin1String("none") ? tr("suggested, not confirmed")
                                                       //: The end of the speaker summary: kept, not confirmed
                                                       : tr("not confirmed");
    return speakers.join(QStringLiteral(" · ")) + QStringLiteral(" · ") + said;
}
void SpeakersController::setGroup(int output, int group) { setField(output, QStringLiteral("group"), std::clamp(group, 0, 15)); }

QJsonObject SpeakersController::draftLayout() const {
    QJsonArray outs;
    for (const QVariant &v : std::as_const(state()->rows)) {
        const QVariantMap m = v.toMap();
        if (!m.value(QStringLiteral("included")).toBool()) continue;
        QJsonObject row{ { QStringLiteral("output"), m.value(QStringLiteral("output")).toInt() },
                         { QStringLiteral("role"), m.value(QStringLiteral("role")).toString() },
                         { QStringLiteral("speaker"), m.value(QStringLiteral("speaker")).toString() },
                         { QStringLiteral("group"), m.value(QStringLiteral("group")).toInt() } };
        // -- WP ADETQ -- whose word each row is, and the detection's record, kept
        const QString source = m.value(QStringLiteral("source")).toString();
        row.insert(QStringLiteral("source"), source.isEmpty() ? QStringLiteral("user") : source);
        outs.append(row);
    }
    QJsonObject layout{ { QStringLiteral("outputs"), outs } };
    if (const QJsonValue d = state()->stored.value(QLatin1String("detected")); d.isObject()) layout.insert(QStringLiteral("detected"), d);
    // -- WP ROLES -- the words the roles were set from, and their checks
    if (const QJsonValue d = state()->stored.value(QLatin1String("stated")); d.isObject()) layout.insert(QStringLiteral("stated"), d);
    return layout;
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
                                 { QStringLiteral("group"), r.value(QLatin1String("group")).toInt() },
                                 { QStringLiteral("source"), str(r, "source") } });   // -- WP ADETQ --
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
    // -- WP ADETQ -- (AUTODES 5.1) detected, not confirmed: its roles guard the outputs meanwhile
    if (s == QLatin1String("autoDetected")) {
        const QJsonObject d = state()->stored.value(QLatin1String("detected")).toObject();
        const QString who = str(d, "by").trimmed();
        if (!here)
            return who.isEmpty() ? tr("Your speaker layout was detected and is not confirmed. Check it in Settings › Speakers.")
                                 : tr("Your speaker layout was detected by AI and is not confirmed. Check it in Settings › Speakers.");
        const qint64 at = d.value(QLatin1String("atMs")).toVariant().toLongLong();
        const QString when = at > 0 ? QStringLiteral(" ") + stampAt(at) : QString();
        const QString placement = str(d, "placement");
        const QString where = placement == QLatin1String("nearField")    ? tr("microphone close to each driver")
                              : placement == QLatin1String("headphones") ? tr("headphones on a coupler")
                                                                         : tr("microphone at the listening position");
        return who.isEmpty()
                   //: %1 when ("today at 14:02"), with a space before it, or ""; %2 where the microphone was
                   ? tr("Detected, not confirmed: the speaker detection probed this DSPi's outputs%1 (%2). Check each role, "
                        "correct what is wrong, then confirm.").arg(when, where)
                   //: %1 the AI agent's name, %2 when ("today at 14:02"), with a space before it, or ""; %3 where the microphone was
                   : tr("Detected by AI, not confirmed: %1 probed this DSPi's outputs%2 (%3). Check each role, correct what "
                        "is wrong, then confirm.").arg(who, when, where);
    }
    // -- WP ROLES -- (owner 2026-11-23) set by AI from the person's words, not checked
    if (s == QLatin1String("stated")) {
        if (!here) return tr("Your speaker roles were set by AI from your words and are not checked. Check them in Settings › Speakers.");
        const QJsonObject d = state()->stored.value(QLatin1String("stated")).toObject();
        const qint64 at = d.value(QLatin1String("atMs")).toVariant().toLongLong();
        const QString when = at > 0 ? QStringLiteral(" ") + stampAt(at) : QString();
        QString who = str(d, "by").trimmed();
        if (who.startsWith(QStringLiteral("AI · "))) who = who.mid(5);
        if (who.isEmpty()) who = tr("An AI agent");
        QString words = str(d, "words").simplified();
        if (words.size() > 160) words = words.left(159) + QChar(0x2026);
        //: %1 the AI agent's name, %2 when ("today at 14:02"), with a space before it, or ""; %3 the person's words, quoted
        return tr("Set by AI from your words, not checked: %1 set these roles%2 from what you told it (“%3”). Check each "
                  "role, correct what is wrong, then confirm.").arg(who, when, words);
    }
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

// ---- -- WP ADETQ -- The speaker detection's part (AUTODES 2, 3, 5.1) ------------------

QVariantList SpeakersController::rows() const {
    const StatePtr st = state();
    QVariantList out;
    out.reserve(st->rows.size());
    for (const QVariant &v : std::as_const(st->rows)) {
        QVariantMap row = v.toMap();
        row.insert(pillOf(*st, row));
        // -- WP UB7 -- the speakers this row is measured with, by name (else the output's label)
        QStringList with;
        if (inLayout(row))
            for (const QVariant &w : std::as_const(st->rows)) {
                const QVariantMap r = w.toMap();
                if (!inLayout(r) || groupOf(r) != groupOf(row)) continue;
                const QString name = r.value(QStringLiteral("speaker")).toString().trimmed();
                const QString word = name.isEmpty() ? r.value(QStringLiteral("label")).toString() : name;
                bool seen = false;
                for (const QString &x : std::as_const(with)) seen = seen || x.toCaseFolded() == word.toCaseFolded();
                if (!seen) with << word;
            }
        row.insert(QStringLiteral("groupName"), with.join(QStringLiteral(" + ")));
        out.append(row);
    }
    return out;
}

// The web's pillOf (ADETW): a row left out says why (nothing heard, off); a row
// says whose word it is, and how sure the detection was. None on a confirmed
// layout's rows as kept, or with no layout and no detection.
QVariantMap SpeakersController::pillOf(const State &st, const QVariantMap &row) const {
    enum Kind { None, Detected, Unsure, Uncertain, Unknown, Nothing, Off, You, Suggested, Stated, StatedTweeter, NotNamed };   // -- WP ROLES -- the last three
    const int o = row.value(QStringLiteral("output")).toInt();
    const QJsonObject det = detectedIn(st.stored, o);
    const QString klass = str(det, "class");
    QString note = str(det, "note");
    Kind kind = None;
    if (!row.value(QStringLiteral("included")).toBool()) {
        if (klass == QLatin1String("nothing")) kind = Nothing;
        else if (klass == QLatin1String("off")) kind = Off;
        // -- WP ROLES -- an output the person's words did not name: unknown, a tweeter
        else if (stateOf(st.stored) == QLatin1String("stated")) kind = NotNamed;
    } else {
        const QString source = row.value(QStringLiteral("source")).toString();
        const QJsonObject kept = rowIn(st.stored, o);
        const bool unchanged = !kept.isEmpty() && str(kept, "role") == row.value(QStringLiteral("role")).toString() &&
                               str(kept, "source") == source;
        if (st.stored.value(QLatin1String("confirmedByUser")).toBool() && unchanged) kind = None;
        else if (source == QLatin1String("user")) kind = You;
        // -- WP ROLES -- set by AI from the person's words; its check may have found a tweeter's top
        else if (source == QLatin1String("stated") && unchanged)
            kind = str(checkIn(st.stored, o), "verdict") == QLatin1String("topHolds") ? StatedTweeter : Stated;
        else if (source == QLatin1String("detected") && !det.isEmpty()) {
            const QString confidence = str(det, "confidence");
            kind = klass == QLatin1String("unknown")    ? Unknown
                   : confidence == QLatin1String("high")   ? Detected
                   : confidence == QLatin1String("medium") ? Unsure
                                                           : Uncertain;
        } else if (!st.stored.isEmpty() || st.stored.value(QLatin1String("detected")).isObject()) kind = Suggested;
    }
    if (kind == None) return { { QStringLiteral("pill"), QString() }, { QStringLiteral("pillKind"), QString() }, { QStringLiteral("pillReason"), QString() } };
    if (note.isEmpty() && kind == Nothing) note = QStringLiteral("detect.nothing");
    if (note.isEmpty() && kind == Off) note = QStringLiteral("detect.off");
    if (note.isEmpty() && kind == Unknown) note = QStringLiteral("detect.unknown");
    static const char *const words[] = { "",
                                         QT_TR_NOOP("detected"),
                                         QT_TR_NOOP("detected?"),
                                         QT_TR_NOOP("uncertain"),
                                         QT_TR_NOOP("unknown"),
                                         QT_TR_NOOP("nothing heard"),
                                         QT_TR_NOOP("off, not probed"),
                                         QT_TR_NOOP("you"),
                                         QT_TR_NOOP("suggested"),
                                         QT_TR_NOOP("from your words"),
                                         QT_TR_NOOP("tweeter-like"),
                                         QT_TR_NOOP("unknown") };
    static const char *const kinds[] = { "", "detected", "detectedUnsure", "uncertain", "unknown", "nothing", "off", "you", "suggested",
                                         "stated", "statedTweeter", "notNamed" };
    // When the notice table lacks the reason
    static const char *const reasons[] = {
        "",
        QT_TR_NOOP("Found by the speaker detection."),
        QT_TR_NOOP("Found by the speaker detection, not sure: correct it if it is wrong."),
        QT_TR_NOOP("The detection was not sure; it is treated as a tweeter to be safe."),
        QT_TR_NOOP("The probe did not finish this output; it is treated as a tweeter until you set its role."),
        QT_TR_NOOP("Nothing reached the microphone from this output: nothing is wired here, or it does not reach the "
                   "microphone. Left out of the layout."),
        QT_TR_NOOP("The output was off or muted, so the detection did not probe it."),
        QT_TR_NOOP("Set or confirmed by you."),
        QT_TR_NOOP("Suggested from the filters and the routing: nobody's word yet."),
        QT_TR_NOOP("Set by AI from your words, not checked. Correct it if it is wrong, then confirm."),
        QT_TR_NOOP("Set by AI from your words, but its top end holds like a tweeter's: nothing below 1 kHz plays on it "
                   "for an agent, and it is guarded as a tweeter. Check the wiring and the role."),
        QT_TR_NOOP("Not in what you told the AI: unknown, treated as a tweeter until you set its role."),
    };
    QString reason = tr(reasons[kind]);
    if (!note.isEmpty())
        if (auto *notices = Registrar::instance<NoticeCenter>()) reason = notices->sentence(note, reason);
    return { { QStringLiteral("pill"), tr(words[kind]) },
             { QStringLiteral("pillKind"), QString::fromLatin1(kinds[kind]) },
             { QStringLiteral("pillReason"), reason } };
}

QString SpeakersController::stampText() const {
    const QJsonObject &l = state()->stored;
    if (l.isEmpty()) return {};
    // -- WP ROLES --
    if (stateOf(l) == QLatin1String("stated")) {
        const qint64 at = l.value(QLatin1String("stated")).toObject().value(QLatin1String("atMs")).toVariant().toLongLong();
        //: %1 when ("today at 14:02", "on 23/11/2026")
        return at > 0 ? tr("Set by AI from your words, not checked, %1.").arg(stampAt(at)) : tr("Set by AI from your words, not checked.");
    }
    if (stateOf(l) == QLatin1String("autoDetected")) {
        const QJsonObject d = l.value(QLatin1String("detected")).toObject();
        const qint64 at = d.value(QLatin1String("atMs")).toVariant().toLongLong();
        const bool ai = !str(d, "by").trimmed().isEmpty();
        if (at <= 0) return ai ? tr("Detected by AI, not confirmed.") : tr("Detected, not confirmed.");
        //: %1 when ("today at 14:02", "on 23/11/2026")
        return ai ? tr("Detected by AI, not confirmed, %1.").arg(stampAt(at)) : tr("Detected, not confirmed, %1.").arg(stampAt(at));
    }
    const qint64 at = stampOf(l);
    if (at <= 0) return {};
    //: %1 when ("today at 14:02", "on 23/11/2026")
    return l.value(QLatin1String("confirmedByUser")).toBool() ? tr("Confirmed %1.").arg(stampAt(at))
                                                               : tr("Kept, not confirmed, %1.").arg(stampAt(at));
}

QVariantMap SpeakersController::detection() const {
    const StatePtr st = state();
    const QJsonObject d = st->stored.value(QLatin1String("detected")).toObject();
    if (d.isEmpty()) return {};
    static const QHash<QString, const char *> classes{
        { QStringLiteral("tweeter"), QT_TR_NOOP("Tweeter") },    { QStringLiteral("midrange"), QT_TR_NOOP("Midrange") },
        { QStringLiteral("woofer"), QT_TR_NOOP("Woofer") },      { QStringLiteral("sub"), QT_TR_NOOP("Subwoofer") },
        { QStringLiteral("fullRange"), QT_TR_NOOP("Full range") }, { QStringLiteral("nothing"), QT_TR_NOOP("Nothing heard") },
        { QStringLiteral("unknown"), QT_TR_NOOP("Unknown, treated as a tweeter") },
        { QStringLiteral("off"), QT_TR_NOOP("Off, not probed") },
    };
    static const QHash<QString, const char *> heard{
        { QStringLiteral("top"), QT_TR_NOOP("4–16 kHz") },
        { QStringLiteral("aboveFloor"), QT_TR_NOOP("above the floor") },
        { QStringLiteral("belowFloor"), QT_TR_NOOP("below the floor") },
        { QStringLiteral("never"), QT_TR_NOOP("never") },
    };
    QVariantList rows;
    for (const QJsonValue &v : d.value(QLatin1String("outputs")).toArray()) {
        const QJsonObject o = v.toObject();
        const int n = o.value(QLatin1String("output")).toInt();
        QString label = tr("Out %1").arg(n + 1);
        for (const QVariant &out : std::as_const(st->outputs))
            if (out.toMap().value(QStringLiteral("output")).toInt() == n) label = out.toMap().value(QStringLiteral("label")).toString();
        const QString klass = str(o, "class");
        const char *cw = classes.value(klass);
        QString words = cw ? tr(cw) : klass;
        const bool sure = str(o, "confidence") == QLatin1String("high");
        if (!sure && klass != QLatin1String("nothing") && klass != QLatin1String("off") && klass != QLatin1String("unknown"))
            words += QLatin1Char('?');
        const char *hw = heard.value(str(o, "heard"));
        const QJsonValue fall = o.value(QLatin1String("topFallDb"));
        rows.append(QVariantMap{
            { QStringLiteral("output"), n },
            { QStringLiteral("label"), label },
            { QStringLiteral("klass"), words },
            { QStringLiteral("confidence"), str(o, "confidence") },
            { QStringLiteral("heard"), klass == QLatin1String("off") ? tr("not probed") : hw ? tr(hw) : QString() },
            //: A level difference in dB, one decimal
            { QStringLiteral("topFall"), fall.isDouble() ? tr("%1 dB").arg(QLocale().toString(fall.toDouble(), 'f', 1)) : QStringLiteral("—") },
            { QStringLiteral("fLow"), hzWords(o.value(QLatin1String("fLowHz"))) },
            { QStringLiteral("lowest"), hzWords(o.value(QLatin1String("lowestHz"))) },
        });
    }
    return { { QStringLiteral("by"), str(d, "by").trimmed() },
             { QStringLiteral("atMs"), d.value(QLatin1String("atMs")).toDouble() },
             { QStringLiteral("stopped"), d.value(QLatin1String("stopped")).toBool() },
             { QStringLiteral("summary"), str(d, "summary") },
             { QStringLiteral("placement"), str(d, "placement") },
             { QStringLiteral("rows"), rows } };
}

bool SpeakersController::detecting() const {
    const auto *measure = Registrar::instance<MeasureController>();
    return measure && measure->session()->detecting();
}

QString SpeakersController::detectBlocker() const {
    if (state()->serial.isEmpty()) return tr("No device connected.");
    if (!m_editable) return tr("This connection may not change the DSPi.");
    // (AUTODES owner question 7: not over a confirmed layout in v1)
    if (status() == QLatin1String("confirmed"))
        return tr("Your speaker layout is confirmed, so the speakers are not detected over it. Correct the roles yourself here.");
    const auto *measure = Registrar::instance<MeasureController>();
    return measure ? measure->session()->detectBlocker() : tr("The Measure view is not available.");
}

QString SpeakersController::detectText() const {
    const auto *measure = Registrar::instance<MeasureController>();
    return measure ? measure->session()->detectText() : QString();
}

QString SpeakersController::detectHeading() const {
    const DeviceSession *s = m_ctx.activeSession();
    //: %1 a device's name
    return tr("Detect the speakers on %1?").arg(s ? s->title() : tr("this DSPi"));
}

// "Out 1–2, 5": as dspi-mcp's risk text names outputs (outs_range_words)
static QString outsWords(const QList<int> &outputs) {
    QStringList runs;
    for (qsizetype i = 0; i < outputs.size();) {
        qsizetype j = i;
        while (j + 1 < outputs.size() && outputs[j + 1] == outputs[j] + 1) ++j;
        runs << (i == j ? QString::number(outputs[i] + 1)
                        : QStringLiteral("%1–%2").arg(outputs[i] + 1).arg(outputs[j] + 1));
        i = j + 1;
    }
    //: Outputs by number: %1 "1–2, 5"
    return SpeakersController::tr("Out %1").arg(runs.join(QStringLiteral(", ")));
}

// AUTODES 4.1 without the agent's and the grant's sentences (4.4); the bullet
// in the words of the agent's question (dspi-mcp's detect risk)
QString SpeakersController::detectQuestion() const {
    const DeviceSession *s = m_ctx.activeSession();
    const QString device = s ? s->title() : tr("this DSPi");
    QList<int> on, off;
    ChannelListModel *ch = m_ctx.device->channels();
    for (int r = 0; r < ch->rowCount(); ++r) {
        if (ch->get(r, QStringLiteral("kind")).toString() != QLatin1String("output")) continue;
        (ch->get(r, QStringLiteral("outputEnabled")).toBool() ? on : off) << ch->get(r, QStringLiteral("number")).toInt();
    }
    const auto *measure = Registrar::instance<MeasureController>();
    const QString placement = measure ? measure->session()->placement() : QString();
    const bool headphones = placement == QLatin1String("headphones");
    const QString from = headphones ? tr("from the headphones on the coupler")
                         : placement == QLatin1String("nearField") ? tr("from the microphone close to each driver")
                                                                   : tr("from the microphone at the listening position");
    const QString list = on.isEmpty() ? tr("its outputs") : outsWords(on);
    //: %1 a device's name, %2 its outputs ("Out 1–6"), %3 where the microphone is ("from the microphone at the listening position")
    QStringList parts{ tr("This finds out what is wired to %1's outputs (%2) by playing quiet test sweeps on each output in turn, %3.")
                           .arg(device, list, from) };
    const QString offWords = off.isEmpty() ? QString()
                             : off.size() == 1
                                 //: %1 an output that is off ("Out 3")
                                 ? tr(" (%1 is off and is not probed)").arg(outsWords(off))
                                 //: %1 outputs that are off ("Out 3–8")
                                 : tr(" (%1 are off and are not probed)").arg(outsWords(off));
    //: %1 the outputs ("Out 1–2"), %2 those that are off, with a space before them, or ""; %3 and %4 levels ("−24 dBFS", "−50 dBFS")
    parts << QStringLiteral("• ") +
                 // -- WP SHOWFIX -- and the free descent, as the core plays it (measure/detect.rs C2)
                 // -- WP FIN015 -- its gate at 177 Hz whatever the floor, its level and floor
                 tr("Speaker detection: %1%2. Each output is first played only above 1 kHz, at most %3. Only an output whose "
                    "response says it is not a tweeter (or that is not heard above its floor) is played lower, and then at "
                    "most %4, no lower than 157 Hz; one still flat at 177 Hz there, as no tweeter is, is then played down "
                    "to 20 Hz (never under the limits profile's high-pass minimum for it) at the level its top was heard "
                    "at, at most %3 (%4 when its top was not heard). Whatever cannot be told apart is treated as a "
                    "tweeter.")
                     .arg(list, offWords, headphones ? QStringLiteral("−36 dBFS") : QStringLiteral("−24 dBFS"),
                          headphones ? QStringLiteral("−60 dBFS") : QStringLiteral("−50 dBFS"));
    parts << QStringLiteral("⚠ ") + AiController::detectWarning();
    parts << tr("Nothing has played yet. Sound cannot be taken back once it plays; Stop ends it.");
    return parts.join(QStringLiteral("\n\n"));
}

void SpeakersController::detect() {
    if (const QString why = detectBlocker(); !why.isEmpty()) {
        say(state(), why, true);
        return;
    }
    say(state(), QString(), false);
    m_detectAsked = true;
    if (auto *measure = Registrar::instance<MeasureController>()) measure->session()->startDetect();
    emit detectChanged();
}

void SpeakersController::stopDetect() {
    if (auto *measure = Registrar::instance<MeasureController>(); measure && measure->session()->detecting()) measure->session()->stop();
}

bool SpeakersController::heardNothing(int output) const {
    const StatePtr st = state();
    return str(detectedIn(st->stored, output), "class") == QLatin1String("nothing") && rowIn(st->stored, output).isEmpty();
}
