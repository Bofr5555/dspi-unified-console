#include "InstallationController.h"

#include "Core.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"   // -- WP MDQT2b --
#include "DeviceSession.h"
#include "LinkController.h"
#include "Registrar.h"

#include <QCoreApplication>
#include <QJSEngine>
#include <QJsonDocument>
#include <QJsonValue>

extern "C" {
#include "dspi_core.h"
}

static const Registrar::Controller<InstallationController> installationController;

namespace {

// Two whole models; a reply that does not fit is asked for again (a read or a
// pure call), or said (the save)
constexpr uint32_t kBuffer = 256 * 1024;
constexpr int kPollMs = 5000;

QJsonObject parse(QByteArray &buf, uint32_t need) {
    buf.truncate(int(need) - 1);
    return QJsonDocument::fromJson(buf).object();
}

QJsonObject call(const std::function<uint32_t(char *, uint32_t)> &fill) {
    QByteArray buf(int(kBuffer), '\0');
    uint32_t need = fill(buf.data(), kBuffer);
    if (need > kBuffer) {
        buf.resize(int(need));
        need = fill(buf.data(), need);
    }
    if (need == 0 || need > uint32_t(buf.size())) return QJsonObject();
    return parse(buf, need);
}

bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }
QString str(const QJsonObject &o, const char *key) { return o.value(QLatin1String(key)).toString(); }
QByteArray json(const QJsonObject &o) { return QJsonDocument(o).toJson(QJsonDocument::Compact); }

QStringList strings(const QJsonValue &v) {
    QStringList out;
    for (const QJsonValue &s : v.toArray()) out << s.toString();
    return out;
}

// The model's lists and what their elements are
struct ListKind {
    const char *list;
    const char *kind;
};
constexpr ListKind kLists[] = { { "rooms", "room" },     { "zones", "zone" },      { "groups", "group" }, { "sources", "source" },
                                { "senders", "sender" }, { "fanOuts", "fan_out" }, { "units", "unit" },   { "scenes", "scene" } };

// A new S/PDIF source's time-base slot until the person chooses (SystemController's: 7-8)
constexpr int kNewSourceMask = 0b1000;

QString roleOf(const QJsonObject &u) {
    const QString transport = str(u, "transport");
    if (transport == QLatin1String("usb_direct")) return QStringLiteral("usb_direct");
    if (transport == QLatin1String("spdif"))
        return u.value(QLatin1String("spdifSource")).toBool() ? QStringLiteral("spdif_source") : QStringLiteral("spdif_follower");
    return u.value(QLatin1String("repeater")).toBool() ? QStringLiteral("repeater") : QStringLiteral("receiver");
}

QJsonObject op(const char *name, std::initializer_list<QPair<QString, QJsonValue>> fields) {
    QJsonObject o(fields);
    o.insert(QStringLiteral("op"), QLatin1String(name));
    return o;
}

// "" as the core's none
QJsonValue orNull(const QString &s) { return s.isEmpty() ? QJsonValue() : QJsonValue(s); }

} // namespace

InstallationController::InstallationController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_core(ctx.core->handle()), m_device(ctx.device), m_registry(ctx.registry), m_session(ctx.activeSession()) {
    m_pool.setMaxThreadCount(1);
    m_pool.setExpiryTimeout(-1);
    m_timer.setInterval(kPollMs);
    connect(&m_timer, &QTimer::timeout, this, &InstallationController::refresh);
    connect(m_device, &DeviceModel::snapshotApplied, this, &InstallationController::updatePolling);
    // -- WP MDQT2b -- with SystemController
    if (m_registry) connect(m_registry, &DeviceRegistry::activeChanged, this, &InstallationController::updatePolling);
    // No wait here, as SystemController: LinkController's quit ends a request
    // under way; the destructor waits afterwards
    connect(qApp, &QCoreApplication::aboutToQuit, this, [this] {
        m_shutDown = true;
        m_timer.stop();
    });
}

InstallationController::~InstallationController() {
    m_shutDown = true;
    m_pool.waitForDone();
}

InstallationController *InstallationController::create(QQmlEngine *, QJSEngine *) {
    InstallationController *self = Registrar::instance<InstallationController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

QString InstallationController::clean(const QString &text) {
    QString out;
    for (const char32_t c : text.toUcs4()) {
        switch (QChar::category(c)) {
        case QChar::Other_Control:
        case QChar::Other_Format:
        case QChar::Other_Surrogate:
        case QChar::Separator_Line:
        case QChar::Separator_Paragraph:
            break;
        default:
            out += QChar::fromUcs4(c);
        }
    }
    return out;
}

bool InstallationController::writable() const { return !m_readOnlyForTest && !m_device->readOnly(); }

void InstallationController::setReadOnlyForTest(bool on) {
    m_readOnlyForTest = on;
    updatePolling();
}

QString InstallationController::editNote() const {
    if (!m_available || mayEdit()) return QString();
    if (!writable()) return tr("This DSPi is read only on this console (its firmware is not this version's).");
    return tr("Only an Admin of the hub may change the installation.");
}

// ---- Reading --------------------------------------------------------------------

LinkController *InstallationController::link() {
    if (!m_link) m_link = Registrar::instance<LinkController>();
    return m_link;
}

// The hub this console is on: its id, its address for one that gives none
QString InstallationController::hubKey() {
    const QString id = link()->hubId();
    return id.isEmpty() ? link()->sessionUrl() : id;
}

// -- WP LAY5b -- (was setShown: the last of two panels to say won)
void InstallationController::panelShown(bool shown) {
    const bool was = m_panels > 0;
    m_panels = qMax(0, m_panels + (shown ? 1 : -1));
    if (was == (m_panels > 0)) return;
    emit shownChanged();
    updatePolling();
}

// Reads only while the panel shows on a hub's device. Off the hub nothing is
// shown; a draft with edits waits for the same hub (a link that came back),
// and applyView drops it on another
void InstallationController::updatePolling() {
    if (m_registry && m_registry->active() != m_session) rebind();   // -- WP MDQT2b --
    if (writable() != m_wasWritable) {
        m_wasWritable = writable();
        emit viewChanged();
        emit editableChanged();
    }
    const bool onHub = m_device->connected() && m_device->remote();
    const bool read = onHub && m_panels > 0 && !m_shutDown;
    if (read && !m_timer.isActive()) {
        m_timer.start();
        refresh();
    } else if (!read && m_timer.isActive()) {
        m_timer.stop();
    }
    if (!onHub && m_available) {
        m_available = false;
        m_error.clear();
        setMessage(QString(), false);
        emit viewChanged();
        emit editableChanged();
    }
}

// -- WP MDQT2b -- Another device is the active one: its core from now on;
// the view and the draft of the one left go (never one hub's edits saved to
// another), and an answer still on its way is dropped
void InstallationController::rebind() {
    m_session = m_registry->active();
    m_core = m_session ? m_session->handle() : nullptr;
    ++m_epoch;
    m_timer.stop();
    m_available = false;
    m_error.clear();
    m_role.clear();
    m_hub.clear();
    m_seen = QJsonArray();
    m_latest = QJsonObject();
    m_dirty = false;
    m_current = false;
    m_stale = false;
    dropLost();   // -- WP LAY5b --
    setMessage(QString(), false);
    if (m_core) takeDraft(QJsonObject());
    m_current = false;
    emit viewChanged();
    emit editableChanged();
}

void InstallationController::refresh() {
    if (m_shutDown || m_reading || !m_core) return;
    m_reading = true;
    FfiCore *core = m_core;
    const QString hub = hubKey();
    const quint64 epoch = m_epoch;   // -- WP MDQT2b --
    m_pool.start([this, core, hub, epoch] {
        const QJsonObject o = call([core](char *b, uint32_t n) { return dspi_installation_view(core, b, n); });
        QMetaObject::invokeMethod(
            this,
            [this, o, hub, epoch] {
                m_reading = false;
                if (epoch != m_epoch) {   // -- WP MDQT2b -- the device left's
                    if (m_timer.isActive()) refresh();
                    return;
                }
                applyView(o, hub);
            },
            Qt::QueuedConnection);
    });
}

void InstallationController::applyView(const QJsonObject &o, const QString &hub) {
    // Asked of a hub this console has left meanwhile
    if (m_shutDown || hub != hubKey() || !m_device->connected() || !m_device->remote()) return;
    if (!isOk(o)) {
        m_error = o.isEmpty() ? tr("The hub did not answer.") : clean(str(o, "error"));
        if (str(o, "code") == QLatin1String("disconnected")) m_available = false;
        emit viewChanged();
        emit editableChanged();
        return;
    }
    if (hub != m_hub) {
        // Another hub's installation: never the last one's edits on it
        m_hub = hub;
        m_dirty = false;
        m_current = false;
        dropLost();   // -- WP LAY5b --
        setMessage(QString(), false);
    }
    m_available = true;
    m_error.clear();
    m_role = str(o, "role");
    m_mayEdit = o.value(QLatin1String("mayEdit")).toBool();
    m_inventory = o.value(QLatin1String("inventory")).toBool();
    m_seen = o.value(QLatin1String("unseen")).toArray();
    m_latest = o.value(QLatin1String("draft")).toObject();
    if (!m_dirty || !m_current) {
        takeDraft(m_latest);
    } else {
        if (m_latest.value(QLatin1String("revision")) != m_draft.value(QLatin1String("revision")) && !m_stale) {
            m_stale = true;
            emit draftChanged();
        }
        updateUnseen();
    }
    emit viewChanged();
    emit editableChanged();
}

// A draft of the installation as read, nothing edited
void InstallationController::takeDraft(const QJsonObject &draft) {
    const bool same = m_current && !m_dirty && !m_stale && draft == m_draft;
    m_draft = draft;
    m_current = true;
    m_stale = false;
    m_edits = QJsonArray();   // -- WP LAY5b --
    // -- WP LAY5b -- a refused edit's words stay on their field until the draft
    // moves, as they are shown: not only until the next poll
    if (same) {
        updateUnseen();
        return;
    }
    m_refusedWhy.clear();
    draftMoved();
}

void InstallationController::draftMoved() {
    m_dirty = m_draft.value(QLatin1String("model")) != m_draft.value(QLatin1String("base"));
    FfiCore *core = m_core;
    const QByteArray draft = json(m_draft);
    const QJsonObject valid = call([&](char *b, uint32_t n) { return dspi_installation_validate(core, draft.constData(), b, n); });
    const QJsonObject said = call([&](char *b, uint32_t n) { return dspi_installation_diff(core, draft.constData(), b, n); });

    // The hub's validator stops at its first problem: one, on the field its words name
    m_issue.clear();
    m_issueSubject.clear();
    m_issueField.clear();
    m_issueShown.clear();
    const QJsonArray issues = valid.value(QLatin1String("value")).toArray();
    if (!isOk(valid)) {
        m_issue = clean(str(valid, "error"));
    } else if (!issues.isEmpty()) {
        const QJsonObject i = issues.first().toObject();
        const QString words = str(i, "message");
        m_issueSubject = str(i, "subject");
        m_issue = clean(words);
        QString kind;
        const QJsonObject e = find(m_issueSubject, &kind);
        // After the name it starts with: a name is never taken for a field
        const QString lead = m_issueSubject + QStringLiteral(": ");
        const qsizetype at = m_issueSubject.isEmpty() ? -1 : words.indexOf(lead);
        QString rest = at < 0 ? words : words.mid(at + lead.size());
        if (kind == QLatin1String("unit") || kind == QLatin1String("fan_out")) {
            if (rest.startsWith(QLatin1String("upstream ")) || rest.startsWith(QLatin1String("its feed ")) || rest.startsWith(QLatin1String("it cannot follow"))
                || rest.startsWith(QLatin1String("one S/PDIF role")))
                m_issueField = QStringLiteral("upstream");
            else if (rest.startsWith(QLatin1String("no room")))
                m_issueField = QStringLiteral("room");
            else if (rest.contains(QLatin1String("USB-direct")) || rest.startsWith(QLatin1String("an S/PDIF")))
                m_issueField = QStringLiteral("role");
            // Nothing chosen yet is the common one: say what to do
            if (m_issueField == QLatin1String("upstream") && str(e, "upstream").isEmpty()) rest = tr("Choose what feeds it.");
        }
        m_issueShown = clean(rest);
        if (!m_issueShown.isEmpty()) m_issueShown[0] = m_issueShown.at(0).toUpper();
    }
    m_changes.clear();
    for (const QString &s : strings(said.value(QLatin1String("value")))) m_changes << clean(s);

    const QJsonObject m = model();
    const auto ids = [&m](std::initializer_list<const char *> lists) {
        QStringList out;
        for (const char *l : lists)
            for (const QJsonValue &v : m.value(QLatin1String(l)).toArray()) out << str(v.toObject(), "id");
        return out;
    };
    const auto update = [this](const QStringList &now, QStringList &into, void (InstallationController::*signal)()) {
        if (now == into) return;
        into = now;
        emit(this->*signal)();
    };
    ++m_generation;
    update(ids({ "rooms" }), m_roomIds, &InstallationController::roomIdsChanged);
    update(ids({ "senders", "fanOuts" }), m_feederIds, &InstallationController::feederIdsChanged);
    update(ids({ "units" }), m_unitIds, &InstallationController::unitIdsChanged);
    update(ids({ "zones" }), m_zoneIds, &InstallationController::zoneIdsChanged);
    update(ids({ "groups" }), m_groupIds, &InstallationController::groupIdsChanged);
    updateUnseen();
    emit draftChanged();
    emit editableChanged();
}

// The units the hub sees that the draft does not hold: joined ones leave the
// list, one the draft removed that the saved installation holds comes back
// after the save, when the hub's answer lists it
void InstallationController::updateUnseen() {
    QStringList held;
    const QJsonObject m = model();
    for (const char *l : { "units", "senders", "fanOuts" })
        for (const QJsonValue &v : m.value(QLatin1String(l)).toArray()) held << str(v.toObject(), "serial");
    QVariantList now;
    QStringList serials;
    for (const QJsonValue &v : m_seen) {
        const QJsonObject u = v.toObject();
        const QString serial = str(u, "serial");
        if (held.contains(serial)) continue;
        serials << serial;
        const QString name = clean(str(u, "name"));
        now.append(QVariantMap{ { QStringLiteral("serial"), serial },
                                { QStringLiteral("name"), name.isEmpty() ? clean(serial) : name },
                                { QStringLiteral("shownSerial"), clean(serial) },
                                { QStringLiteral("online"), u.value(QLatin1String("online")).toBool() },
                                { QStringLiteral("outputs"), u.value(QLatin1String("outputs")).toInt() } });
    }
    if (now == m_unseen) return;
    m_unseen = now;
    ++m_generation;
    emit unseenChanged();
    emit draftChanged();
    if (serials == m_unseenSerials) return;
    m_unseenSerials = serials;
    emit unseenSerialsChanged();
}

QVariantMap InstallationController::unseenUnit(const QString &serial, int) const {
    for (const QVariant &v : m_unseen)
        if (v.toMap().value(QStringLiteral("serial")).toString() == serial) return v.toMap();
    return {};
}

// ---- The draft, for QML ------------------------------------------------------------

QJsonObject InstallationController::find(const QString &id, QString *kind) const {
    if (id.isEmpty()) return {};
    const QJsonObject m = model();
    for (const ListKind &k : kLists)
        for (const QJsonValue &v : m.value(QLatin1String(k.list)).toArray()) {
            const QJsonObject e = v.toObject();
            if (str(e, "id") != id) continue;
            if (kind) *kind = QLatin1String(k.kind);
            return e;
        }
    return {};
}

QString InstallationController::label(const QString &id) const {
    const QString name = clean(str(find(id), "name"));
    return name.isEmpty() ? clean(id) : name;
}

QVariantMap InstallationController::element(const QString &id, int) const {
    QString kind;
    const QJsonObject e = find(id, &kind);
    if (e.isEmpty()) return {};
    QVariantMap out{ { QStringLiteral("id"), id },
                     { QStringLiteral("kind"), kind },
                     { QStringLiteral("name"), label(id) },
                     { QStringLiteral("serial"), clean(str(e, "serial")) },
                     { QStringLiteral("hubSerial"), str(e, "serial") },   // -- WP LAY5b --
                     { QStringLiteral("upstream"), str(e, "upstream") },
                     { QStringLiteral("source"), str(e, "source") },
                     { QStringLiteral("rooms"), strings(e.value(QLatin1String("rooms"))) },
                     { QStringLiteral("units"), strings(e.value(QLatin1String("units"))) } };
    if (kind == QLatin1String("unit")) {
        out.insert(QStringLiteral("room"), str(e, "room"));
        out.insert(QStringLiteral("role"), roleOf(e));
        out.insert(QStringLiteral("spdifSource"), e.value(QLatin1String("spdifSource")).toBool());
        out.insert(QStringLiteral("txMask"), e.value(QLatin1String("spdifTxMask")).toInt());
    }
    return out;
}

QVariantList InstallationController::roomChoices(int) const {
    QVariantList out{ QVariantMap{ { QStringLiteral("id"), QString() }, { QStringLiteral("name"), tr("No room") } } };
    for (const QString &id : m_roomIds) out.append(QVariantMap{ { QStringLiteral("id"), id }, { QStringLiteral("name"), label(id) } });
    return out;
}

QVariantList InstallationController::sourceChoices(int) const {
    QVariantList out{ QVariantMap{ { QStringLiteral("id"), QString() }, { QStringLiteral("name"), tr("No source") } } };
    for (const QJsonValue &v : model().value(QLatin1String("sources")).toArray()) {
        const QString id = str(v.toObject(), "id");
        out.append(QVariantMap{ { QStringLiteral("id"), id }, { QStringLiteral("name"), label(id) } });
    }
    return out;
}

QVariantList InstallationController::feedChoices(const QString &role, const QString &selfId, const QString &value, int) const {
    QVariantList out;
    if (value.isEmpty()) out.append(QVariantMap{ { QStringLiteral("id"), QString() }, { QStringLiteral("name"), tr("Choose…") } });
    bool found = value.isEmpty();
    const auto add = [&](const QString &id) {
        if (id == selfId) return;
        found = found || id == value;
        out.append(QVariantMap{ { QStringLiteral("id"), id }, { QStringLiteral("name"), label(id) } });
    };
    const bool spdif = role == QLatin1String("spdif_follower");
    if (!spdif)
        for (const QString &id : m_feederIds) add(id);
    for (const QJsonValue &v : model().value(QLatin1String("units")).toArray()) {
        const QJsonObject u = v.toObject();
        if (spdif ? u.value(QLatin1String("spdifSource")).toBool() : roleOf(u) == QLatin1String("repeater")) add(str(u, "id"));
    }
    // What it names now, though it cannot feed it (removed, another role): the issue says so
    if (!found)
        out.prepend(QVariantMap{ { QStringLiteral("id"), value },
                                 { QStringLiteral("name"), find(value).isEmpty() ? tr("%1 (removed)").arg(clean(value)) : label(value) } });
    return out;
}

QString InstallationController::issueOf(const QString &id, const QString &field, int) const {
    if (!m_refusedWhy.isEmpty() && id == m_refusedId && field == m_refusedField) return m_refusedWhy;
    if (!m_issueShown.isEmpty() && !m_issueSubject.isEmpty() && id == m_issueSubject && field == m_issueField) return m_issueShown;
    return {};
}

// ---- The edits -------------------------------------------------------------------

bool InstallationController::refuse(const QString &id, const QString &field, const QString &why) {
    m_refusedId = id;
    m_refusedField = field;
    m_refusedWhy = why;
    // The rows show the draft's value again
    ++m_generation;
    emit draftChanged();
    return false;
}

bool InstallationController::edit(const QJsonArray &edits, const QString &id, const QString &field) {
    if (!editable()) return false;
    FfiCore *core = m_core;
    const QByteArray draft = json(m_draft);
    const QByteArray list = QJsonDocument(edits).toJson(QJsonDocument::Compact);
    QJsonObject o = call([&](char *b, uint32_t n) { return dspi_installation_draft_edit(core, draft.constData(), list.constData(), b, n); });
    if (!isOk(o)) {
        // "edit 1: no room "x"": which of them is no matter here
        QString why = str(o, "error");
        const qsizetype at = why.indexOf(QLatin1String("edit "));
        const qsizetype colon = at < 0 ? -1 : why.indexOf(QLatin1String(": "), at);
        if (colon >= 0) why = why.mid(colon + 2);
        why = clean(why);
        if (!why.isEmpty()) why[0] = why.at(0).toUpper();
        return refuse(id, field, why.isEmpty() ? tr("This cannot be done.") : why);
    }
    o.remove(QStringLiteral("ok"));
    m_draft = o;
    for (const QJsonValue &e : edits) m_edits.append(e);   // -- WP LAY5b --
    m_refusedWhy.clear();
    setMessage(QString(), false);
    draftMoved();
    return true;
}

bool InstallationController::join(const QString &serial, const QString &name, const QString &as, const QString &room, const QString &upstream) {
    const QString at = QStringLiteral("serial:") + serial;
    const QString called = name.trimmed();
    if (serial.isEmpty()) return refuse(at, QStringLiteral("serial"), tr("Give its serial."));
    if (as == QLatin1String("sender"))
        return edit({ op("addSender", { { QStringLiteral("name"), called }, { QStringLiteral("serial"), serial } }) }, at, QString());
    if (as == QLatin1String("fan_out")) {
        if (upstream.isEmpty()) return refuse(at, QStringLiteral("upstream"), tr("Choose what feeds it."));
        return edit({ op("addFanOut", { { QStringLiteral("name"), called }, { QStringLiteral("serial"), serial }, { QStringLiteral("upstream"), upstream } }) },
                    at, QStringLiteral("upstream"));
    }
    const bool source = as == QLatin1String("spdif_source");
    const bool fed = as != QLatin1String("usb_direct") && !source;
    const QJsonObject before = m_draft;
    const QJsonArray editsBefore = m_edits;   // -- WP LAY5b --
    if (!edit({ op("addUnit", { { QStringLiteral("serial"), serial },
                                { QStringLiteral("name"), called },
                                { QStringLiteral("room"), orNull(room) },
                                { QStringLiteral("role"), source ? QStringLiteral("spdif_follower") : as },
                                { QStringLiteral("upstream"), fed ? orNull(upstream) : QJsonValue() } }) },
              at, QString()))
        return false;
    if (!source) return true;
    // The source's mark needs the id the core made
    for (const QString &id : m_unitIds)
        if (str(find(id), "serial") == serial
            && edit({ op("setSpdifSource", { { QStringLiteral("id"), id }, { QStringLiteral("txMask"), kNewSourceMask } }) }, at, QString()))
            return true;
    const QString why = m_refusedWhy;
    m_draft = before;
    m_edits = editsBefore;
    draftMoved();
    return refuse(at, QString(), why);
}

bool InstallationController::rename(const QString &id, const QString &name) {
    const QString to = name.trimmed();
    // As it is shown: nothing typed
    if (to == label(id)) return true;
    return edit({ op("renameElement", { { QStringLiteral("id"), id }, { QStringLiteral("name"), to } }) }, id, QStringLiteral("name"));
}

bool InstallationController::setRoom(const QString &id, const QString &room) {
    return edit({ op("setUnitRoom", { { QStringLiteral("id"), id }, { QStringLiteral("room"), orNull(room) } }) }, id, QStringLiteral("room"));
}

bool InstallationController::setRole(const QString &id, const QString &role) {
    const QString was = roleOf(find(id));
    if (role == was) return true;
    QJsonArray edits;
    // A source by its transport stops being one; a unit on the audio link that
    // is a source beside stays one (the core drops it where it cannot be)
    if (was == QLatin1String("spdif_source"))
        edits.append(op("setSpdifSource", { { QStringLiteral("id"), id }, { QStringLiteral("txMask"), 0 } }));
    const bool source = role == QLatin1String("spdif_source");
    edits.append(op("setUnitRole", { { QStringLiteral("id"), id }, { QStringLiteral("role"), source ? QStringLiteral("spdif_follower") : role } }));
    if (source) {
        const int mask = find(id).value(QLatin1String("spdifTxMask")).toInt();
        edits.append(op("setSpdifSource", { { QStringLiteral("id"), id }, { QStringLiteral("txMask"), mask ? mask : kNewSourceMask } }));
    }
    return edit(edits, id, QStringLiteral("role"));
}

bool InstallationController::setUpstream(const QString &id, const QString &upstream) {
    if (upstream.isEmpty()) return refuse(id, QStringLiteral("upstream"), tr("Choose what feeds it."));
    return edit({ op("setUnitUpstream", { { QStringLiteral("id"), id }, { QStringLiteral("upstream"), upstream } }) }, id, QStringLiteral("upstream"));
}

bool InstallationController::setSpdifSlot(const QString &id, int slot, bool on) {
    if (slot < 0 || slot > 3) return false;
    const QJsonObject u = find(id);
    if (!u.value(QLatin1String("spdifSource")).toBool()) return false;
    int mask = u.value(QLatin1String("spdifTxMask")).toInt();
    mask = on ? (mask | (1 << slot)) : (mask & ~(1 << slot));
    if (mask == 0) return refuse(id, QString(), tr("An S/PDIF source sends its time base on one pair of outputs at least."));
    return edit({ op("setSpdifSource", { { QStringLiteral("id"), id }, { QStringLiteral("txMask"), mask } }) }, id, QString());
}

bool InstallationController::setMember(const QString &id, const QString &member, bool in) {
    QString kind;
    const QJsonObject e = find(id, &kind);
    const bool zone = kind == QLatin1String("zone");
    if (!zone && kind != QLatin1String("group")) return false;
    QStringList members = strings(e.value(QLatin1String(zone ? "rooms" : "units")));
    if (members.contains(member) == in) return true;
    if (in) members << member;
    else members.removeAll(member);
    const QJsonArray list = QJsonArray::fromStringList(members);
    return edit({ zone ? op("setZoneRooms", { { QStringLiteral("id"), id }, { QStringLiteral("rooms"), list } })
                       : op("setGroupUnits", { { QStringLiteral("id"), id }, { QStringLiteral("units"), list } }) },
                id, QString());
}

bool InstallationController::addRoom(const QString &name) {
    const QString to = name.trimmed();
    if (to.isEmpty()) return refuse(QStringLiteral("new:room"), QString(), tr("Give it a name."));
    return edit({ op("addRoom", { { QStringLiteral("name"), to } }) }, QStringLiteral("new:room"), QString());
}

bool InstallationController::addZone(const QString &name, const QString &source) {
    const QString to = name.trimmed();
    if (to.isEmpty()) return refuse(QStringLiteral("new:zone"), QString(), tr("Give it a name."));
    return edit({ op("addZone", { { QStringLiteral("name"), to }, { QStringLiteral("source"), orNull(source) } }) }, QStringLiteral("new:zone"), QString());
}

bool InstallationController::addGroup(const QString &name) {
    const QString to = name.trimmed();
    if (to.isEmpty()) return refuse(QStringLiteral("new:group"), QString(), tr("Give it a name."));
    return edit({ op("addGroup", { { QStringLiteral("name"), to } }) }, QStringLiteral("new:group"), QString());
}

bool InstallationController::addSender(const QString &name) {
    const QString to = name.trimmed();
    if (to.isEmpty()) return refuse(QStringLiteral("new:sender"), QString(), tr("Give it a name."));
    return edit({ op("addSender", { { QStringLiteral("name"), to } }) }, QStringLiteral("new:sender"), QString());
}

bool InstallationController::remove(const QString &id) {
    return edit({ op("removeElement", { { QStringLiteral("id"), id } }) }, id, QString());
}

// ---- Saving ----------------------------------------------------------------------

void InstallationController::setMessage(const QString &message, bool failed) {
    if (message == m_message && failed == m_messageFailed) return;
    m_message = message;
    m_messageFailed = failed;
    emit messageChanged();
}

void InstallationController::save() {
    if (!m_dirty || !m_issue.isEmpty() || !editable() || m_hub != hubKey()) return;
    ++m_busy;
    setMessage(QString(), false);
    emit editableChanged();
    FfiCore *core = m_core;
    const QString hub = m_hub;
    const QByteArray draft = json(m_draft);
    const quint64 epoch = m_epoch;   // -- WP MDQT2b --
    m_pool.start([this, core, hub, draft, epoch] {
        // Called once, never again: a reply too long for the buffer means it
        // has happened (fns_misc/installation.rs). Then the installation as it
        // is now, saved or not
        QByteArray buf(int(kBuffer), '\0');
        const uint32_t need = dspi_installation_save(core, draft.constData(), buf.data(), kBuffer);
        const bool tooLong = need > kBuffer;
        const QJsonObject saved = need == 0 || tooLong ? QJsonObject() : parse(buf, need);
        const QJsonObject view = call([core](char *b, uint32_t n) { return dspi_installation_view(core, b, n); });
        QMetaObject::invokeMethod(
            this,
            [this, hub, saved, tooLong, view, epoch] {
                --m_busy;
                // -- WP MDQT2b -- saved on the device left: its words are not this one's
                if (epoch != m_epoch) {
                    emit editableChanged();
                    return;
                }
                const bool sameHub = hub == m_hub && hub == hubKey();
                if (sameHub && (tooLong || isOk(saved))) {
                    // Until the view is read, there is no draft to edit
                    m_dirty = false;
                    m_current = false;
                    dropLost();   // -- WP LAY5b --
                    QStringList said{ tr("Saved on the hub.") };
                    for (const QString &w : strings(saved.value(QLatin1String("warnings")))) said << clean(w);
                    if (said.size() > 1) said.first() = tr("Saved on the hub, with warnings:");
                    setMessage(said.join(QLatin1Char('\n')), false);
                    emit draftChanged();
                } else if (sameHub) {
                    if (str(saved, "code") == QLatin1String("conflict") || str(saved, "noticeKey") == QLatin1String("dlink.conflict")) {
                        // `stale` says it: the draft is made anew on Reload
                        m_stale = true;
                        emit draftChanged();
                    } else {
                        setMessage(saved.isEmpty() ? tr("The hub did not answer.") : clean(str(saved, "error")), true);
                    }
                }
                emit editableChanged();
                applyView(view, hub);
            },
            Qt::QueuedConnection);
    });
}

void InstallationController::reload() {
    if (m_busy > 0) return;
    // -- WP LAY5b -- edits that could not be saved are kept aside (the web's
    // "Make them again"); edits of one's own discarded are not
    if (m_stale && m_dirty && !m_edits.isEmpty()) {
        m_lost = m_edits;
        m_lostChanges = m_changes;
    }
    m_dirty = false;
    setMessage(QString(), false);
    if (!m_latest.isEmpty()) takeDraft(m_latest);
    refresh();
}

// -- WP LAY5b -- On the fresh draft, never a copy of the old one over it
bool InstallationController::remake() {
    if (m_lost.isEmpty() || !editable()) return false;
    if (!edit(m_lost, QString(), QString())) {
        //: %1 is the core's reason
        setMessage(tr("They could not be made again: %1").arg(m_refusedWhy), true);
        return false;
    }
    dropLost();
    return true;
}

void InstallationController::dropLost() {
    if (m_lost.isEmpty()) return;
    m_lost = QJsonArray();
    m_lostChanges.clear();
    emit draftChanged();
}
