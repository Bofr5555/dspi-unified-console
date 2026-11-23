// -- WP ATQT -- Tools > Measure…'s "Several units" tab (UnitsMeasureController.h).
#include "UnitsMeasureController.h"

#include "AppIdentity.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "LinkController.h"
#include "LinkSession.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "MeasureController.h"
#include "SessionModel.h"
#include "SpeakersController.h"   // -- WP RCFIX --
#include "NoticeCenter.h"
#include "AiController.h"   // -- WP MCP5 --
#include "Shell.h"          // -- WP MCP5 --

#include "CoreApi.h"

#include <QCoreApplication>
#include <QJsonDocument>
#include <QPointer>
#include <QRegularExpression>
#include <QSysInfo>
#include <algorithm>
#include <cmath>

static const Registrar::Controller<UnitsMeasureController> unitsMeasureController;

namespace {

constexpr int kOnceBytes = 8 << 20;
constexpr int kReadBytes = 1 << 18;

QString str(const QJsonObject &o, const char *key) { return o.value(QLatin1String(key)).toString(); }
bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }
QByteArray compact(const QJsonObject &o) { return QJsonDocument(o).toJson(QJsonDocument::Compact); }
QByteArray compact(const QJsonArray &a) { return QJsonDocument(a).toJson(QJsonDocument::Compact); }
QJsonObject object(const QByteArray &t) { return QJsonDocument::fromJson(t).object(); }

// A call that changes something: once, with room for any answer
QByteArray once(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(kOnceBytes, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

// A failure the core did not give (a unit's connection, the core silent)
QByteArray failure(const QString &text, const QString &key = QString()) {
    QJsonObject o{ { QStringLiteral("ok"), false }, { QStringLiteral("code"), QStringLiteral("invalid") }, { QStringLiteral("error"), text } };
    if (!key.isEmpty()) o.insert(QStringLiteral("noticeKey"), key);
    return compact(o);
}

QString sentence(QString s) {
    if (!s.isEmpty()) s[0] = s[0].toUpper();
    return s;
}

// The hub's open refusals (the Link error's notice key) as the step's rows
QString stepKey(const QString &key) {
    if (key == QLatin1String("dlink.not_one_domain")) return QStringLiteral("measure.system.not_one_domain");
    if (key == QLatin1String("dlink.not_locked")) return QStringLiteral("measure.system.not_locked");
    if (key == QLatin1String("dlink.no_timeline")) return QStringLiteral("measure.system.no_timeline");
    return key;
}

// -- WP RCFIX -- (review RC, RC3) output `output`'s role in a unit's stored layout,
// "" unless the user confirmed it
QString confirmedRole(const QJsonObject &layout, int output) {
    if (!layout.value(QLatin1String("confirmedByUser")).toBool()) return {};
    for (const QJsonValue &v : layout.value(QLatin1String("outputs")).toArray())
        if (v.toObject().value(QLatin1String("output")).toInt() == output) return str(v.toObject(), "role");
    return {};
}

QString orFull(const QString &role) { return role.isEmpty() ? QStringLiteral("full") : role; }

double num(const QJsonValue &v) { return v.isDouble() ? v.toDouble() : NAN; }

QString usText(double v) {
    if (!std::isfinite(v)) return QStringLiteral("–");
    return QCoreApplication::translate("UnitsMeasureController", "%1 µs").arg(v, 0, 'f', 1);
}

} // namespace

// A unit's own connection to the hub. -- WP XDEV1 -- (MPLAN 1.2 item 6) There is
// one session per serial: a unit open in a tab is judged by that tab's core
// (`tab` set; nothing here closes it), any other by an extra core of the set
// (`extra`: its id), closed with the last owner (on any thread: the close is
// the registry's, on the GUI thread). Either core stays valid after its device
// closed (the set retires it), so a loop job still holding one is safe.
struct UnitsMeasureController::UnitCore {
    FfiCore *core = nullptr;
    QPointer<DeviceSession> tab;
    QString extra;
    UnitCore(FfiCore *c, DeviceSession *t, const QString &id) : core(c), tab(t), extra(id) {}
    UnitCore(const UnitCore &) = delete;
    UnitCore &operator=(const UnitCore &) = delete;
    ~UnitCore() {
        if (extra.isEmpty()) return;
        QCoreApplication *app = QCoreApplication::instance();
        if (!app) return;
        QMetaObject::invokeMethod(app, [id = extra] {
            if (DeviceRegistry *r = DeviceRegistry::instance()) r->closeExtra(id);
        }, Qt::QueuedConnection);
    }
};

QByteArray UnitsMeasureController::coreCall(FfiCore *c, const QString &op, const QByteArray &a, const QByteArray &b) {
    const auto read = [](const std::function<uint32_t(char *, uint32_t)> &f) { return MeasureController::reply(f, kReadBytes); };
    if (op == QLatin1String("unitCheck"))
        return read([&](char *buf, uint32_t l) { return dspi_measure_system_unit_check(c, a.constData(), b.constData(), buf, l); });
    if (op == QLatin1String("preflight"))
        return read([&](char *buf, uint32_t l) { return dspi_measure_system_preflight(c, a.constData(), b.constData(), buf, l); });
    if (op == QLatin1String("start"))
        return once([&](char *buf, uint32_t l) { return dspi_measure_system_start(c, a.constData(), b.constData(), buf, l); });
    if (op == QLatin1String("sumCheck"))
        return once([&](char *buf, uint32_t l) { return dspi_measure_system_sum_check(c, a.constData(), buf, l); });
    if (op == QLatin1String("verify"))
        return once([&](char *buf, uint32_t l) { return dspi_measure_system_verify(c, a.constData(), buf, l); });
    if (op == QLatin1String("apply"))
        return once([&](char *buf, uint32_t l) { return dspi_measure_system_apply(c, a.toUInt(), buf, l); });
    if (op == QLatin1String("keep")) return once([&](char *buf, uint32_t l) { return dspi_measure_system_keep(c, buf, l); });
    if (op == QLatin1String("revert")) return once([&](char *buf, uint32_t l) { return dspi_measure_system_revert(c, buf, l); });
    if (op == QLatin1String("stop")) return once([&](char *buf, uint32_t l) { return dspi_measure_system_stop(c, buf, l); });
    if (op == QLatin1String("applied")) return read([&](char *buf, uint32_t l) { return dspi_measure_system_applied(c, buf, l); });
    if (op == QLatin1String("state")) return read([&](char *buf, uint32_t l) { return dspi_measure_system_state(c, buf, l); });
    if (op == QLatin1String("result"))
        return MeasureController::reply([&](char *buf, uint32_t l) { return dspi_measure_system_result(c, buf, l); }, 1 << 22);
    if (op == QLatin1String("proposal"))
        return read([&](char *buf, uint32_t l) { return dspi_measure_system_proposal(c, buf, l); });
    // -- review AT-D2 -- the demo hub's simulated microphone (WP ATDEMO)
    if (op == QLatin1String("simInfo")) return read([&](char *buf, uint32_t l) { return dspi_measure_system_sim_info(c, buf, l); });
    if (op == QLatin1String("simSet"))
        return once([&](char *buf, uint32_t l) { return dspi_measure_system_sim_set(c, a == "1", buf, l); });
    return failure(QStringLiteral("unknown call %1").arg(op));
}

// ---- Life ------------------------------------------------------------------------------

UnitsMeasureController::UnitsMeasureController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    m_pool.setMaxThreadCount(1);
    m_pool.setExpiryTimeout(-1);
    m_guardTimer.setInterval(1000);
    connect(&m_guardTimer, &QTimer::timeout, this, [this] {
        emit guardChanged();
        if (guardRemainingS() > 0) return;
        m_guardTimer.stop();
        rehost();   // -- WP MDQT2c --
    });
    m_keepTimer.setInterval(250);
    connect(&m_keepTimer, &QTimer::timeout, this, [this] {
        emit keepChanged();
        if (!m_question || keepReady()) m_keepTimer.stop();
    });
    connect(qApp, &QCoreApplication::aboutToQuit, this, [this] {
        m_shutDown = true;
        closeAll();
    });
    // The other controllers exist once every constructor has run
    QMetaObject::invokeMethod(this, [this] {
        rehost();   // -- WP MDQT2c --
        // -- WP MCP5 -- an AI agent's runs across units
        if (auto *ai = Registrar::instance<AiController>())
            connect(ai, &AiController::measureRequested, this, &UnitsMeasureController::agentRequest);
        // -- WP RCFIX -- this console's own DSPi: its confirmed layout changed
        if (auto *sc = Registrar::instance<SpeakersController>())
            connect(sc, &SpeakersController::storedChanged, this, [this] {
                if (m_entered && !running()) refreshUnits();
            });
        if (auto *link = Registrar::instance<LinkController>()) {
            connect(link, &LinkController::devicesChanged, this, [this] {
                if (m_entered) refreshUnits();
            });
            connect(link, &LinkController::statusChanged, this, [this] {
                emit availabilityChanged();
                emit proposalChanged();
            });
        }
    }, Qt::QueuedConnection);
    // -- WP MDQT2c -- its device closed while it ran (after MeasureController let go)
    connect(ctx.registry, &DeviceRegistry::sessionClosing, this, &UnitsMeasureController::rehost, Qt::QueuedConnection);
    // -- WP XDEV1 -- a unit measured on its tab's core: the tab closed
    connect(ctx.registry, &DeviceRegistry::sessionClosing, this, &UnitsMeasureController::tabClosing);
    connect(ctx.registry, &DeviceRegistry::extrasChanged, this, &UnitsMeasureController::openWaiting);   // -- WP LAY7 --
    connect(ctx.device, &DeviceModel::snapshotApplied, this, [this] {
        rehost();   // -- WP MDQT2c --
        if (m_entered) refreshUnits();
    });
}

UnitsMeasureController::~UnitsMeasureController() {
    m_shutDown = true;
    m_cores.clear();
    m_pool.waitForDone();
}

UnitsMeasureController *UnitsMeasureController::create(QQmlEngine *, QJSEngine *) {
    return Registrar::instance<UnitsMeasureController>();
}

// -- WP MDQT2c -- The device whose core the session runs on: the active one, but
// while the session runs, or an applied one waits in its guard, the one it was
// started on (until it is closed)
MeasureSession *UnitsMeasureController::measure() const {
    if (m_host) return m_host;
    auto *measure = Registrar::instance<MeasureController>();
    return measure ? measure->session() : nullptr;
}

LinkSession *UnitsMeasureController::hostLink() const {
    MeasureSession *m = measure();
    return m ? m->link() : nullptr;
}

void UnitsMeasureController::rehost() {
    auto *measure = Registrar::instance<MeasureController>();
    if (!measure || m_shutDown) return;
    MeasureSession *now = measure->session();
    emit hostChanged();
    if (m_host == now) return;
    const bool was = m_hosted;
    if (m_host && (running() || m_starting || m_micOurs || guardRemainingS() > 0)) return;
    if (m_host && m_host->loop()) disconnect(m_host->loop(), &MeasureLoop::systemState, this, nullptr);
    m_host = now;
    m_hosted = true;
    emit hostChanged();
    if (now->loop()) connect(now->loop(), &MeasureLoop::systemState, this, &UnitsMeasureController::stateArrived);
    if (!was) return;
    // Another device's core and hub: what the last one's said is not its
    ++m_epoch;
    disarm();
    closeAll();
    m_starting = false;
    m_micOurs = false;
    setBusy(QString());
    m_placement.clear();
    m_units.clear();
    m_state = {};
    m_measured.clear();
    m_issueText.clear();
    m_issueKey.clear();
    m_preflightLines.clear();
    setProposal({});
    setApplied({});
    m_result = {};
    m_arrivals.clear();
    m_delays.clear();
    m_graph.clear();
    emit formChanged();
    emit preflightChanged();
    emit stateChanged();
    emit resultChanged();
    clearWords();
    if (m_entered) enter();
}

QString UnitsMeasureController::heldWords() const {
    MeasureSession *m = measure();
    if (!m || m->isActive() || !m->session()) return {};
    //: %1 is another open device
    return tr("This is the measurement across units started on %1.").arg(m->session()->title());
}

QString UnitsMeasureController::ownSerial() const {
    MeasureSession *m = measure();
    return m && m->connected() ? m->model()->serial() : QString();
}

QString UnitsMeasureController::role() const {
    if (!m_roleForTest.isEmpty()) return m_roleForTest;
    LinkSession *link = hostLink();
    return link && link->inSession() ? link->role() : QString();
}

void UnitsMeasureController::onLoop(std::function<QByteArray(FfiCore *)> job, std::function<void(const QByteArray &)> done) {
    MeasureSession *m = measure();
    if (!m || !m->loop() || m_shutDown) {
        if (done) done(failure(tr("The measurement is not available.")));
        return;
    }
    m->onLoop(this, std::move(job), std::move(done));
}

// ---- Words -----------------------------------------------------------------------------

QString UnitsMeasureController::noticeTitle(const QString &key) const {
    auto *n = Registrar::instance<NoticeCenter>();
    return n && !key.isEmpty() ? n->row(key).value(QStringLiteral("title")).toString() : QString();
}

void UnitsMeasureController::clearWords() {
    m_error.clear();
    m_errorKey.clear();
    m_said.clear();
    emit wordsChanged();
}

void UnitsMeasureController::clearError() { clearWords(); }

void UnitsMeasureController::failWords(const QString &text, const QString &key) {
    m_error = text;
    m_errorKey = key;
    m_said.clear();
    emit wordsChanged();
}

// A refusal in words: its notice's title (the hub's open refusals as the
// step's rows: "These units do not share a clock"), then the core's words
void UnitsMeasureController::fail(const QJsonObject &reply, const QString &step) {
    const QString key = stepKey(str(reply, "noticeKey"));
    QString text;
    if (reply.isEmpty()) text = tr("The core did not answer.");
    else {
        // The core's and the client's prefixes say nothing to a person (as the
        // web's): "DSPi Link: hub refused: not_one_domain the units ..."
        static const QRegularExpression prefix(QStringLiteral("^(invalid argument|DSPi Link|unsupported): "));
        static const QRegularExpression hub(QStringLiteral("^hub refused: [a-z_]+ ?"), QRegularExpression::CaseInsensitiveOption);
        QString msg = sentence(str(reply, "error").remove(prefix).remove(hub));
        const QString title = noticeTitle(key);
        if (!title.isEmpty() && !key.startsWith(QLatin1String("core.")) && title != msg)
            //: A refusal: %1 its notice's title, %2 the core's or the hub's words
            text = msg.isEmpty() ? title : tr("%1: %2").arg(title, msg);
        else text = msg.isEmpty() ? MeasureController::failureText(reply) : msg;
    }
    failWords(text, key);
    if (!step.isEmpty()) emit stepDone(step, false);
}

void UnitsMeasureController::say(const QString &text) {
    // -- WP ATQTSIM -- the core's words begin in lower case ("applied: verify it ..."): a sentence here
    m_said = sentence(text);
    m_error.clear();
    m_errorKey.clear();
    emit wordsChanged();
}

void UnitsMeasureController::setBusy(const QString &what) {
    if (what == m_busy) return;
    m_busy = what;
    emit wordsChanged();
}

QString UnitsMeasureController::outputLabel(const QString &serial, int output) const {
    return tr("%1 output %2").arg(serial).arg(output + 1);
}

QString UnitsMeasureController::issueWords(const QJsonObject &i) const {
    if (i.isEmpty()) return {};
    const QString title = noticeTitle(str(i, "key"));
    const QString msg = sentence(str(i, "message"));
    // The core's words may begin with the title already
    // -- WP ATQTSIM -- or say it further on (a verify that did not pass: "...; the verify did not
    // confirm the alignment; Keep it only if ..."): then the title is not said twice
    return !title.isEmpty() && !msg.contains(title, Qt::CaseInsensitive) ? tr("%1: %2").arg(title, msg) : msg;
}

QString UnitsMeasureController::arrivalWords(const QJsonObject &a) const {
    const QJsonValue u = a.value(QLatin1String("uncertaintyUs"));
    QString s = usText(num(a.value(QLatin1String("arrivalUs"))));
    if (u.isDouble()) s += tr(" ± %1").arg(usText(u.toDouble()));
    return s;
}

// ---- The tab ---------------------------------------------------------------------------

void UnitsMeasureController::enter() {
    m_entered = true;
    LinkSession *link = hostLink();
    if (!link || !link->inSession()) {
        m_unavailable = tr("Measuring several units together needs the DSPi hub: it starts every unit's generator on one "
                           "shared sample counter.");
        emit availabilityChanged();
        emit stepDone(QStringLiteral("enter"), true);
        return;
    }
    // The hub's family: the core's preflight says when it needs the DSPi hub
    const QByteArray probe = R"({"units":[{"serial":"-","outputs":[{"output":0}]},{"serial":"--","outputs":[{"output":0}]}]})";
    Calls calls = m_calls;
    onLoop([calls, probe](FfiCore *c) {
        const QByteArray pre = calls(c, QStringLiteral("preflight"), probe, "[]");
        const QByteArray st = calls(c, QStringLiteral("state"), {}, {});
        const QByteArray ap = calls(c, QStringLiteral("applied"), {}, {});
        const QByteArray res = calls(c, QStringLiteral("result"), {}, {});
        const QByteArray pro = calls(c, QStringLiteral("proposal"), {}, {});
        const QByteArray sim = calls(c, QStringLiteral("simInfo"), {}, {});   // -- review AT-D2 --
        QJsonArray all;
        for (const QByteArray &x : { pre, st, ap, res, pro, sim }) all.append(object(x));
        return compact(all);
    },
           [this](const QByteArray &r) {
               const QJsonArray all = QJsonDocument::fromJson(r).array();
               const QJsonObject pre = all.at(0).toObject();
               m_unavailable.clear();
               for (const QJsonValue &b : pre.value(QLatin1String("blockers")).toArray())
                   if (str(b.toObject(), "key") == QLatin1String("measure.system.needs_hub"))
                       m_unavailable = tr("Measuring several units together needs the DSPi hub: it starts every unit's "
                                          "generator on one shared sample counter.");
               emit availabilityChanged();
               // -- review AT-D2 -- the demo hub's simulated microphone, where the hub has one
               const QJsonObject sim = all.at(5).toObject();
               m_simAvailable = isOk(sim) && sim.value(QLatin1String("available")).toBool();
               m_simOn = m_simAvailable && sim.value(QLatin1String("on")).toBool();
               m_simLabel = str(sim, "label");
               emit simChanged();
               refreshUnits();
               // What the core still holds from earlier on this console
               const QJsonObject st = all.at(1).toObject();
               if (isOk(st)) {
                   m_state = st;
                   emit stateChanged();
               }
               if (isOk(all.at(2).toObject())) setApplied(all.at(2).toObject());
               if (isOk(all.at(3).toObject())) setResult(all.at(3).toObject());
               if (isOk(all.at(4).toObject())) setProposal(all.at(4).toObject());
               emit stepDone(QStringLiteral("enter"), m_unavailable.isEmpty());
           });
}

void UnitsMeasureController::leave() {
    m_entered = false;
    ++m_epoch;
    if (running()) stop();
    closeAll();
}

// -- WP RCFIX -- The stored layout of this console's own DSPi, {} while it is not
// the one Settings › Speakers shows
QJsonObject UnitsMeasureController::ownLayout() const {
    auto *sc = Registrar::instance<SpeakersController>();
    const QJsonObject layout = sc ? sc->stored() : QJsonObject();
    return str(layout, "serial") == ownSerial() ? layout : QJsonObject();
}

void UnitsMeasureController::refreshUnits() {
    LinkSession *link = hostLink();
    const QVariantList devices = link ? link->devices() : QVariantList();
    const QString own = ownSerial();
    QVariantList rows;
    for (const QVariant &d : devices) {
        const QVariantMap dev = d.toMap();
        const QString serial = dev.value(QStringLiteral("serial")).toString();
        if (serial.isEmpty()) continue;
        QVariantMap row = unitRow(serial);
        const QString name = dev.value(QStringLiteral("name")).toString();
        row.insert(QStringLiteral("serial"), serial);
        row.insert(QStringLiteral("name"), name.isEmpty() ? serial : name);
        row.insert(QStringLiteral("online"), dev.value(QStringLiteral("state")).toString() == QLatin1String("online"));
        row.insert(QStringLiteral("own"), serial == own);
        if (!row.contains(QStringLiteral("chosen"))) row.insert(QStringLiteral("chosen"), false);
        if (!row.contains(QStringLiteral("opening"))) row.insert(QStringLiteral("opening"), false);
        if (!row.contains(QStringLiteral("error"))) row.insert(QStringLiteral("error"), QString());
        // This console's own DSPi: its outputs are the Measure window's
        if (serial == own && measure()) {
            QVariantList outs;
            const QVariantList was = row.value(QStringLiteral("outputs")).toList();
            for (const QVariant &o : measure()->outputs()) {
                QVariantMap out = o.toMap();
                const int n = out.value(QStringLiteral("output")).toInt();
                out.insert(QStringLiteral("on"), n == 0 && row.value(QStringLiteral("chosen")).toBool());
                // -- WP RCFIX -- the layout's role, again when it changes
                const QString confirmed = confirmedRole(ownLayout(), n);
                out.insert(QStringLiteral("layoutRole"), confirmed);
                out.insert(QStringLiteral("role"), orFull(confirmed));
                for (const QVariant &w : was)
                    if (w.toMap().value(QStringLiteral("output")).toInt() == n) {
                        out.insert(QStringLiteral("on"), w.toMap().value(QStringLiteral("on")));
                        if (w.toMap().value(QStringLiteral("layoutRole")).toString() == confirmed)
                            out.insert(QStringLiteral("role"), w.toMap().value(QStringLiteral("role")));
                    }
                outs.append(out);
            }
            row.insert(QStringLiteral("outputs"), outs);
        } else if (!row.contains(QStringLiteral("outputs"))) {
            row.insert(QStringLiteral("outputs"), QVariantList());
        }
        rows.append(row);
    }
    if (rows == m_units) return;
    m_units = rows;
    emit formChanged();
}

QVariantMap UnitsMeasureController::unitRow(const QString &serial) const {
    for (const QVariant &v : m_units)
        if (v.toMap().value(QStringLiteral("serial")).toString() == serial) return v.toMap();
    return {};
}

void UnitsMeasureController::setUnitField(const QString &serial, const QString &field, const QVariant &value) {
    for (QVariant &v : m_units) {
        QVariantMap row = v.toMap();
        if (row.value(QStringLiteral("serial")).toString() != serial) continue;
        row.insert(field, value);
        v = row;
        emit formChanged();
        return;
    }
}

// ---- The form --------------------------------------------------------------------------

void UnitsMeasureController::setPlacement(const QString &p) {
    if (p == m_placement) return;
    m_placement = p;
    m_preflightLines.clear();
    emit preflightChanged();
    emit formChanged();
}

// The coordinator's headphone-safety rule: nothing plays before the person
// said where the microphone is; units are measured together with their
// speakers at the listening position only
QString UnitsMeasureController::placementProblem() const {
    if (m_placement.isEmpty()) return tr("Choose where the microphone is first: nothing plays before you have.");
    if (m_placement == QLatin1String("headphones"))
        return tr("Headphones are never measured across units: measure them one unit at a time on the Measure tab, at "
                  "headphone levels.");
    if (m_placement != QLatin1String("listeningPosition"))
        return tr("Units are measured together at the listening position only: put the microphone there, or measure one "
                  "unit at a time on the Measure tab.");
    return {};
}

void UnitsMeasureController::setSweepMs(int ms) {
    ms = std::clamp(ms, 1000, 15000);
    if (ms == m_sweepMs) return;
    m_sweepMs = ms;
    emit formChanged();
}

void UnitsMeasureController::setPasses(int n) {
    if (n != 0) n = std::clamp(n, 2, 8);
    if (n == m_passes) return;
    m_passes = n;
    emit formChanged();
}

void UnitsMeasureController::setAnchor(const QString &a) {
    if (a == m_anchor) return;
    m_anchor = a;
    emit formChanged();
}

QVariantList UnitsMeasureController::anchors() const {
    QVariantList list{ QVariantMap{ { QStringLiteral("id"), QString() }, { QStringLiteral("label"), tr("The first unit's first output") } } };
    for (const QVariant &v : m_units) {
        const QVariantMap u = v.toMap();
        if (!u.value(QStringLiteral("chosen")).toBool()) continue;
        const QString serial = u.value(QStringLiteral("serial")).toString();
        for (const QVariant &o : u.value(QStringLiteral("outputs")).toList()) {
            const QVariantMap out = o.toMap();
            if (!out.value(QStringLiteral("on")).toBool()) continue;
            const int n = out.value(QStringLiteral("output")).toInt();
            list.append(QVariantMap{ { QStringLiteral("id"), QStringLiteral("%1:%2").arg(serial).arg(n) },
                                     { QStringLiteral("label"), outputLabel(serial, n) } });
        }
    }
    return list;
}

void UnitsMeasureController::setRunName(const QString &n) {
    const QString t = n.left(64);
    if (t == m_runName) return;
    m_runName = t;
    emit formChanged();
}

// What keeps the form from being checked: the hub, the placement, two units,
// an output on each, their connections
QString UnitsMeasureController::blocker() const {
    if (!m_unavailable.isEmpty()) return m_unavailable;
    const QString p = placementProblem();
    if (!p.isEmpty()) return p;
    int chosen = 0;
    for (const QVariant &v : m_units) {
        const QVariantMap u = v.toMap();
        if (!u.value(QStringLiteral("chosen")).toBool()) continue;
        ++chosen;
        const QString serial = u.value(QStringLiteral("serial")).toString();
        bool any = false;
        for (const QVariant &o : u.value(QStringLiteral("outputs")).toList()) any = any || o.toMap().value(QStringLiteral("on")).toBool();
        if (u.value(QStringLiteral("opening")).toBool()) return tr("%1: its own connection is opening.").arg(serial);
        if (!u.value(QStringLiteral("own")).toBool() && !m_cores.contains(serial))
            return u.value(QStringLiteral("error")).toString().isEmpty()
                       ? tr("%1: its own connection is not open (choose it again).").arg(serial)
                       : u.value(QStringLiteral("error")).toString();
        if (!any) return tr("Choose at least one output of %1.").arg(serial);
    }
    if (chosen < 2) return tr("Choose at least two units: one unit is measured on the Measure tab.");
    return {};
}

void UnitsMeasureController::setUnitChosen(const QString &serial, bool on) {
    QVariantMap row = unitRow(serial);
    if (row.isEmpty() || row.value(QStringLiteral("chosen")).toBool() == on || running()) return;
    if (on && !row.value(QStringLiteral("online")).toBool()) return;
    row.insert(QStringLiteral("chosen"), on);
    QVariantList outs = row.value(QStringLiteral("outputs")).toList();
    for (int i = 0; i < outs.size(); ++i) {
        QVariantMap o = outs.at(i).toMap();
        o.insert(QStringLiteral("on"), on && o.value(QStringLiteral("output")).toInt() == 0);
        o.insert(QStringLiteral("role"), orFull(o.value(QStringLiteral("layoutRole")).toString()));   // -- WP RCFIX --
        outs[i] = o;
    }
    row.insert(QStringLiteral("outputs"), outs);
    row.insert(QStringLiteral("error"), QString());
    for (QVariant &v : m_units)
        if (v.toMap().value(QStringLiteral("serial")).toString() == serial) v = row;
    if (!on && m_anchor.startsWith(serial + QLatin1Char(':'))) m_anchor.clear();
    m_preflightLines.clear();
    emit preflightChanged();
    emit formChanged();
    if (row.value(QStringLiteral("own")).toBool()) {
        emit stepDone(QStringLiteral("unit"), true);
        return;
    }
    if (on) openUnit(serial);
    else closeUnit(serial);
}

void UnitsMeasureController::setOutputOn(const QString &serial, int output, bool on) {
    QVariantMap row = unitRow(serial);
    if (row.isEmpty() || running()) return;
    QVariantList outs = row.value(QStringLiteral("outputs")).toList();
    for (int i = 0; i < outs.size(); ++i) {
        QVariantMap o = outs.at(i).toMap();
        if (o.value(QStringLiteral("output")).toInt() != output) continue;
        o.insert(QStringLiteral("on"), on);
        outs[i] = o;
    }
    if (!on && m_anchor == QStringLiteral("%1:%2").arg(serial).arg(output)) m_anchor.clear();
    setUnitField(serial, QStringLiteral("outputs"), outs);
    m_preflightLines.clear();
    emit preflightChanged();
}

void UnitsMeasureController::setOutputRole(const QString &serial, int output, const QString &role) {
    static const QStringList roles{ QStringLiteral("full"), QStringLiteral("sub"), QStringLiteral("low"), QStringLiteral("mid"),
                                    QStringLiteral("high") };
    QVariantMap row = unitRow(serial);
    if (row.isEmpty() || !roles.contains(role) || running()) return;
    QVariantList outs = row.value(QStringLiteral("outputs")).toList();
    for (int i = 0; i < outs.size(); ++i) {
        QVariantMap o = outs.at(i).toMap();
        if (o.value(QStringLiteral("output")).toInt() != output) continue;
        o.insert(QStringLiteral("role"), role);
        outs[i] = o;
    }
    setUnitField(serial, QStringLiteral("outputs"), outs);
}

// ---- The units' own connections ----------------------------------------------------------

void UnitsMeasureController::openUnit(const QString &serial) {
    LinkSession *link = hostLink();
    if (!link || !link->inSession()) {
        setUnitField(serial, QStringLiteral("error"), tr("%1: this console is not connected through a hub.").arg(serial));
        emit stepDone(QStringLiteral("unit"), false);
        return;
    }
    if (m_cores.contains(serial)) return;
    DeviceRegistry *registry = m_ctx.registry;
    if (!registry || registry->isShutDown()) {
        setUnitField(serial, QStringLiteral("error"), tr("%1: no connection could be made.").arg(serial));
        emit stepDone(QStringLiteral("unit"), false);
        return;
    }
    setUnitField(serial, QStringLiteral("opening"), true);
    setUnitField(serial, QStringLiteral("error"), QString());
    setUnitField(serial, QStringLiteral("tab"), QString());
    // -- WP LAY7 -- (RCFIX follow-up) chosen again while the core it was judged by
    // is still closing (or opening): it is opened once its serial is free
    if (!registry->holder(serial) && registry->extraSerials().contains(serial)) {
        m_waiting.insert(serial);
        return;
    }
    QPointer<UnitsMeasureController> self(this);
    // -- WP XDEV1 -- open in a tab: that tab's core, no second session
    if (DeviceSession *tab = registry->holder(serial)) {
        readOutputs(serial, std::make_shared<UnitCore>(tab->handle(), tab, QString()), QString());
        return;
    }
    // Not open: an extra core of the set, holding its serial while it is chosen
    const QString name = (QSysInfo::machineHostName().isEmpty() ? AppIdentity::displayName() : QSysInfo::machineHostName())
                         + QStringLiteral(" (") + serial + QLatin1Char(')');
    const QJsonObject target{
        { QStringLiteral("kind"), QStringLiteral("hub") },
        { QStringLiteral("url"), link->sessionUrl() },
        { QStringLiteral("hubId"), link->hubId() },
        { QStringLiteral("serial"), serial },
        { QStringLiteral("clientName"), name },
        { QStringLiteral("appVersion"), AppIdentity::version() },
        { QStringLiteral("credential"), link->sessionCredential() },
        { QStringLiteral("tlsAdvertised"), link->sessionTls() },
    };
    registry->openExtra(target, [self, serial](FfiCore *core, const QString &id, const QString &code, const QString &error) {
        // Closed with it, if nobody here wants it any more
        auto unit = core ? std::make_shared<UnitCore>(core, nullptr, id) : UnitPtr();
        if (!self) return;
        QString why;
        if (!unit) {
            if (code == QLatin1String("needs_pairing"))
                why = tr("%1: the hub asks this console to pair again: open the unit in the console once, then choose it here.").arg(serial);
            else if (code == QLatin1String("full"))
                why = tr("%1: too many devices are open here; close a tab first.").arg(serial);
            else
                why = tr("%1: %2").arg(serial, error.isEmpty() ? tr("no connection could be made.") : error);
        }
        self->readOutputs(serial, unit, why);
    });
}

// -- WP LAY7 -- An extra core was opened or closed: the units that waited for it
void UnitsMeasureController::openWaiting() {
    const QStringList held = m_ctx.registry->extraSerials();
    const QSet<QString> waiting = m_waiting;
    for (const QString &serial : waiting) {
        if (m_cores.contains(serial) || !unitRow(serial).value(QStringLiteral("chosen")).toBool()) {
            m_waiting.remove(serial);   // the open on its way became its connection, or it is not chosen any more
        } else if (!held.contains(serial)) {
            m_waiting.remove(serial);
            openUnit(serial);
        }
    }
}

// The unit's outputs, read on its core off the GUI thread; then it is the
// chosen unit's connection, if it is still chosen
void UnitsMeasureController::readOutputs(const QString &serial, UnitPtr unit, const QString &error) {
    QPointer<UnitsMeasureController> self(this);
    m_pool.start([self, unit, serial, error]() mutable {
        QString why = error;
        QVariantList outputs;
        if (unit && why.isEmpty()) {
            FfiCore *c = unit->core;
            const QJsonObject labels = object(MeasureController::reply([&](char *b, uint32_t l) { return dspi_output_labels(c, b, l); }));
            // -- WP RCFIX -- its rows' roles from its own confirmed layout
            const QJsonObject layout = object(MeasureController::reply([&](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); }));
            if (!isOk(labels)) why = tr("%1: its state could not be read.").arg(serial);
            for (const QJsonValue &v : labels.value(QLatin1String("value")).toArray()) {
                const QJsonObject o = v.toObject();
                const int n = o.value(QLatin1String("output")).toInt();
                outputs.append(QVariantMap{ { QStringLiteral("output"), n },
                                            { QStringLiteral("label"), str(o, "label") },
                                            { QStringLiteral("on"), n == 0 },
                                            { QStringLiteral("layoutRole"), confirmedRole(layout, n) },
                                            { QStringLiteral("role"), orFull(confirmedRole(layout, n)) } });
            }
        }
        if (!why.isEmpty()) unit.reset();
        QMetaObject::invokeMethod(qApp, [self, unit, serial, why, outputs]() mutable {
            if (!self) return;
            UnitsMeasureController *me = self;
            const QVariantMap row = me->unitRow(serial);
            const bool wanted = me->m_entered && !me->m_shutDown && row.value(QStringLiteral("chosen")).toBool();
            me->setUnitField(serial, QStringLiteral("opening"), false);
            // A tab it was read on that closed meanwhile is no connection
            const bool tabGone = unit && unit->extra.isEmpty() && !unit->tab;
            if (!wanted) return;   // an extra core closes with `unit`
            if (!why.isEmpty() || tabGone) {
                me->setUnitField(serial, QStringLiteral("error"), why.isEmpty() ? tr("%1: its tab was closed: choose it again.").arg(serial) : why);
                emit me->stepDone(QStringLiteral("unit"), false);
                return;
            }
            me->m_cores.insert(serial, unit);
            me->m_waiting.remove(serial);   // -- WP LAY7 --
            me->setUnitField(serial, QStringLiteral("tab"), unit->tab ? unit->tab->title() : QString());
            me->setUnitField(serial, QStringLiteral("outputs"), outputs);
            emit me->stepDone(QStringLiteral("unit"), true);
        }, Qt::QueuedConnection);
    });
}

// -- WP XDEV1 -- A tab whose core a chosen unit is judged by goes: the unit
// has no connection now (choosing it again opens an extra one)
void UnitsMeasureController::tabClosing(DeviceSession *session) {
    QStringList gone;
    for (auto it = m_cores.cbegin(); it != m_cores.cend(); ++it)
        if (it.value()->tab == session) gone << it.key();
    for (const QString &serial : std::as_const(gone)) {
        m_cores.remove(serial);
        setUnitField(serial, QStringLiteral("tab"), QString());
        setUnitField(serial, QStringLiteral("error"), tr("%1: its tab was closed: choose it again.").arg(serial));
    }
}

FfiCore *UnitsMeasureController::unitCore(const QString &serial) const {
    const UnitPtr u = m_cores.value(serial);
    return u ? u->core : nullptr;
}

bool UnitsMeasureController::unitInTab(const QString &serial) const {
    const UnitPtr u = m_cores.value(serial);
    return u && u->tab;
}

void UnitsMeasureController::closeUnit(const QString &serial) {
    UnitPtr u = m_cores.take(serial);
    if (m_waiting.remove(serial)) setUnitField(serial, QStringLiteral("opening"), false);   // -- WP LAY7 --
    setUnitField(serial, QStringLiteral("outputs"), QVariantList());
    setUnitField(serial, QStringLiteral("tab"), QString());   // -- WP XDEV1 --
    // Closed off the GUI thread (a loop job may still hold it: then there)
    if (u) m_pool.start([u = std::move(u)]() mutable { u.reset(); });
}

void UnitsMeasureController::closeAll() {
    // -- WP LAY7 --
    for (const QString &s : std::as_const(m_waiting)) setUnitField(s, QStringLiteral("opening"), false);
    m_waiting.clear();
    const QStringList serials = m_cores.keys();
    for (const QString &s : serials) {
        UnitPtr u = m_cores.take(s);
        if (u) m_pool.start([u = std::move(u)]() mutable { u.reset(); });
    }
    for (QVariant &v : m_units) {
        QVariantMap row = v.toMap();
        if (row.value(QStringLiteral("own")).toBool()) continue;
        row.insert(QStringLiteral("chosen"), false);
        row.insert(QStringLiteral("outputs"), QVariantList());
        v = row;
    }
    emit formChanged();
}

// ---- The calls -------------------------------------------------------------------------

QByteArray UnitsMeasureController::setupJson(int rateHz) const {
    QJsonArray units;
    for (const QVariant &v : m_units) {
        const QVariantMap u = v.toMap();
        if (!u.value(QStringLiteral("chosen")).toBool()) continue;
        QJsonArray outs;
        for (const QVariant &o : u.value(QStringLiteral("outputs")).toList()) {
            const QVariantMap out = o.toMap();
            if (!out.value(QStringLiteral("on")).toBool()) continue;
            outs.append(QJsonObject{ { QStringLiteral("output"), out.value(QStringLiteral("output")).toInt() },
                                     { QStringLiteral("role"), out.value(QStringLiteral("role")).toString() } });
        }
        units.append(QJsonObject{ { QStringLiteral("serial"), u.value(QStringLiteral("serial")).toString() }, { QStringLiteral("outputs"), outs } });
    }
    const int colon = int(m_anchor.lastIndexOf(QLatin1Char(':')));
    QJsonObject setup{ { QStringLiteral("units"), units },
                       { QStringLiteral("passes"), m_passes },
                       { QStringLiteral("anchorSerial"), colon > 0 ? m_anchor.left(colon) : QString() },
                       { QStringLiteral("anchorOutput"), colon > 0 ? QJsonValue(m_anchor.mid(colon + 1).toInt()) : QJsonValue() },
                       { QStringLiteral("micDocument"), measure() ? QString::fromUtf8(measure()->micDocument()) : QString() },
                       { QStringLiteral("sweepMs"), m_sweepMs },
                       { QStringLiteral("captureRateHz"), rateHz },
                       { QStringLiteral("name"), m_runName } };
    return compact(setup);
}

// Every unit's fresh check from its own core (review AT-C F2), asked right
// before the call that takes them, in the same job on the measurement loop
void UnitsMeasureController::withChecks(const QString &step, const QByteArray &setup, Job then,
                                        std::function<void(const QJsonObject &)> done) {
    // The units' cores as they are now (the job holds them while it runs)
    QList<QPair<QString, UnitPtr>> cores;
    const QJsonArray units = QJsonDocument::fromJson(setup).object().value(QLatin1String("units")).toArray();
    const QString own = ownSerial();
    for (const QJsonValue &u : units) {
        const QString serial = str(u.toObject(), "serial");
        if (serial == own) {
            cores.append({ serial, nullptr });
            continue;
        }
        UnitPtr c = m_cores.value(serial);
        if (!c) {
            failWords(tr("%1: its own connection is not open yet (choose it again).").arg(serial));
            emit stepDone(step, false);
            return;
        }
        cores.append({ serial, c });
    }
    Calls calls = m_calls;
    onLoop([cores, units, setup, then, calls](FfiCore *main) {
        QJsonArray checks;
        for (int i = 0; i < cores.size(); ++i) {
            FfiCore *c = cores.at(i).second ? cores.at(i).second->core : main;
            const QByteArray unit = compact(units.at(i).toObject());
            QJsonObject check = object(calls(c, QStringLiteral("unitCheck"), setup, unit));
            if (!isOk(check)) {
                // The unit names itself
                const QString why = check.isEmpty() ? QStringLiteral("no answer") : str(check, "error");
                check.insert(QStringLiteral("error"), QStringLiteral("%1: %2").arg(cores.at(i).first, why));
                check.insert(QStringLiteral("ok"), false);
                return compact(check);
            }
            check.remove(QStringLiteral("ok"));
            checks.append(check);
        }
        return then(main, compact(checks));
    },
           [done](const QByteArray &r) { done(object(r)); });
}

void UnitsMeasureController::check() {
    clearWords();
    const QString why = blocker();
    if (!why.isEmpty()) {
        failWords(why, m_placement != QLatin1String("listeningPosition") ? QStringLiteral("measure_system_placement") : QString());
        emit stepDone(QStringLiteral("check"), false);
        return;
    }
    setBusy(tr("Checking every unit"));
    const QByteArray setup = setupJson(48000);
    Calls calls = m_calls;
    withChecks(QStringLiteral("check"), setup,
               [calls, setup](FfiCore *c, const QByteArray &checks) { return calls(c, QStringLiteral("preflight"), setup, checks); },
               [this](const QJsonObject &pre) {
                   setBusy(QString());
                   m_preflightLines.clear();
                   if (!isOk(pre)) {
                       emit preflightChanged();
                       fail(pre, QStringLiteral("check"));
                       return;
                   }
                   if (pre.value(QLatin1String("clear")).toBool())
                       m_preflightLines.append(QVariantMap{ { QStringLiteral("kind"), QStringLiteral("ready") },
                                                            { QStringLiteral("text"), tr("Ready: nothing blocks the measurement across units.") } });
                   for (const char *kind : { "blockers", "warnings" })
                       for (const QJsonValue &v : pre.value(QLatin1String(kind)).toArray()) {
                           const QJsonObject i = v.toObject();
                           m_preflightLines.append(QVariantMap{ { QStringLiteral("kind"), qstrcmp(kind, "blockers") == 0 ? QStringLiteral("blocker") : QStringLiteral("warning") },
                                                                { QStringLiteral("key"), str(i, "key") },
                                                                { QStringLiteral("serial"), str(i, "serial") },
                                                                { QStringLiteral("text"), issueWords(i) } });
                       }
                   emit preflightChanged();
                   emit stepDone(QStringLiteral("check"), true);
               });
}

void UnitsMeasureController::start() {
    clearWords();
    MeasureSession *m = measure();
    const QString why = blocker();
    if (!why.isEmpty() || running() || !m) {
        failWords(why.isEmpty() ? tr("A measurement across units is already running.") : why);
        emit stepDone(QStringLiteral("start"), false);
        return;
    }
    if (!m->isActive()) {   // -- WP MDQT2c --
        failWords(heldWords());
        emit stepDone(QStringLiteral("start"), false);
        return;
    }
    if (!m->playBlocker().isEmpty()) {
        failWords(m->playBlocker());
        emit stepDone(QStringLiteral("start"), false);
        return;
    }
    if (m->running()) {
        failWords(tr("Stop the measurement, the live view or the passive RTA first."));
        emit stepDone(QStringLiteral("start"), false);
        return;
    }
    const quint64 epoch = m_epoch;
    m_starting = true;
    updateRunning();
    setBusy(tr("Starting"));
    // The microphone first, then the checks and the start
    openMicForRun([this, m, epoch](bool ok, int rateHz) {
        if (!ok || epoch != m_epoch) {
            m_starting = false;
            setBusy(QString());
            m->closeMic();
            updateRunning();
            if (ok) emit stepDone(QStringLiteral("start"), false);
            else {
                // -- review AT-D2 -- the simulated microphone's own refusal stays
                if (m_error.isEmpty()) failWords(m->error().isEmpty() ? tr("The microphone could not be opened.") : m->error());
                emit stepDone(QStringLiteral("start"), false);
            }
            return;
        }
        m_micOurs = true;
        const QByteArray setup = setupJson(rateHz);
        Calls calls = m_calls;
        withChecks(QStringLiteral("start"), setup,
                   [calls, setup](FfiCore *c, const QByteArray &checks) -> QByteArray {
                       const QJsonObject pre = object(calls(c, QStringLiteral("preflight"), setup, checks));
                       if (!isOk(pre) || !pre.value(QLatin1String("clear")).toBool())
                           return compact(QJsonObject{ { QStringLiteral("ok"), true }, { QStringLiteral("preflight"), pre } });
                       // Clear: fresh checks again right before the start (the next job)
                       return QByteArray();
                   },
                   [this, m, epoch, setup](const QJsonObject &first) {
                       if (!first.isEmpty() && isOk(first) && first.contains(QLatin1String("preflight"))) {
                           // The preflight found something: shown, nothing started
                           const QJsonObject pre = first.value(QLatin1String("preflight")).toObject();
                           m_starting = false;
                           setBusy(QString());
                           m_micOurs = false;
                           m->closeMic();
                           updateRunning();
                           if (!isOk(pre)) fail(pre, QStringLiteral("start"));
                           else {
                               m_preflightLines.clear();
                               for (const QJsonValue &v : pre.value(QLatin1String("blockers")).toArray()) {
                                   const QJsonObject i = v.toObject();
                                   m_preflightLines.append(QVariantMap{ { QStringLiteral("kind"), QStringLiteral("blocker") },
                                                                        { QStringLiteral("key"), str(i, "key") },
                                                                        { QStringLiteral("serial"), str(i, "serial") },
                                                                        { QStringLiteral("text"), issueWords(i) } });
                               }
                               emit preflightChanged();
                               failWords(tr("Something blocks the measurement across units: see the check."));
                               emit stepDone(QStringLiteral("start"), false);
                           }
                           return;
                       }
                       if (!first.isEmpty() && !isOk(first)) {
                           m_starting = false;
                           setBusy(QString());
                           m_micOurs = false;
                           m->closeMic();
                           updateRunning();
                           fail(first, QStringLiteral("start"));
                           return;
                       }
                       if (epoch != m_epoch) {
                           m_starting = false;
                           setBusy(QString());
                           m_micOurs = false;
                           m->closeMic();
                           updateRunning();
                           emit stepDone(QStringLiteral("start"), false);
                           return;
                       }
                       // The preflight is clear: the fresh checks and the start
                       Calls calls = m_calls;
                       withChecks(QStringLiteral("start"), setup,
                                  [calls, setup](FfiCore *c, const QByteArray &checks) { return calls(c, QStringLiteral("start"), setup, checks); },
                                  [this, m, epoch, setup](const QJsonObject &st) {
                                      m_starting = false;
                                      setBusy(QString());
                                      if (!isOk(st)) {
                                          m_micOurs = false;
                                          m->closeMic();
                                          updateRunning();
                                          fail(st, QStringLiteral("start"));
                                          return;
                                      }
                                      m_measured = setup;
                                      m_result = {};
                                      m_arrivals.clear();
                                      m_delays.clear();
                                      m_graph.clear();
                                      emit resultChanged();
                                      setProposal({});
                                      setApplied({});
                                      disarm();
                                      beginSession(st);
                                      // A Stop or a leave while it started: stopped at once
                                      if (epoch != m_epoch) stop();
                                      emit stepDone(QStringLiteral("start"), true);
                                  });
                   });
    });
}

void UnitsMeasureController::beginSession(const QJsonObject &state) {
    m_state = state;
    m_issueText.clear();
    m_issueKey.clear();
    if (MeasureSession *m = measure(); m && m->loop()) m->loop()->system.store(true);
    emit stateChanged();
    updateRunning();
}

void UnitsMeasureController::stop() {
    ++m_epoch;
    disarm();
    MeasureSession *m = measure();
    if (!m || !m->loop()) return;
    const bool ours = running() || m_micOurs;
    if (!ours) return;
    MeasureLoop *loop = m->loop();
    Calls calls = m_calls;
    onLoop([loop, calls](FfiCore *c) {
        loop->system.store(false);
        return calls(c, QStringLiteral("stop"), {}, {});
    },
           [this](const QByteArray &r) {
               const QJsonObject st = object(r);
               if (isOk(st)) stateArrived(r);
               else endSession();
           });
}

// -- review AT-D2 -- the demo hub's simulated microphone (WP ATDEMO)
void UnitsMeasureController::setUseSim(bool on) {
    if (m_useSim == on || running()) return;
    m_useSim = on;
    emit simChanged();
}

void UnitsMeasureController::openMicForRun(std::function<void(bool, int)> done) {
    MeasureSession *m = measure();
    if (!m) {
        done(false, 0);
        return;
    }
    const bool sim = useSim();
    Calls calls = m_calls;
    // The core is told either way: on, its hub's room is the microphone (the hub
    // answers only for its own demo units, or the session does not start); off,
    // the Measure window's input is, as before.
    onLoop([calls, sim](FfiCore *c) { return calls(c, QStringLiteral("simSet"), sim ? "1" : "0", {}); },
           [this, m, sim, done](const QByteArray &r) {
               const QJsonObject o = object(r);
               m_simOn = isOk(o) && o.value(QLatin1String("on")).toBool();
               emit simChanged();
               if (!sim) {
                   m->openMic(done);
                   return;
               }
               if (!m_simOn) {
                   // Never a real microphone in its place by itself: the person chose the simulated one
                   failWords(isOk(o) ? tr("The simulated microphone could not be turned on.") : MeasureController::failureText(o));
                   done(false, 0);
                   return;
               }
               // Nothing is opened here: the core reads the hub's room on every tick
               done(true, 48000);
           });
}

void UnitsMeasureController::endSession() {
    m_starting = false;
    if (m_micOurs) {
        m_micOurs = false;
        if (MeasureSession *m = measure()) m->closeMic();
    }
    updateRunning();
}

void UnitsMeasureController::stateArrived(const QByteArray &json) {
    const QJsonObject st = object(json);
    if (st.isEmpty() || !isOk(st)) return;
    const bool was = m_state.value(QLatin1String("active")).toBool();
    m_state = st;
    const QJsonObject issue = st.value(QLatin1String("issue")).toObject();
    if (!st.value(QLatin1String("active")).toBool() && !issue.isEmpty()) {
        // A unit's time base moving ends a run; the hub may see it first and the
        // core then reports the hub's stop (or the hub as gone) with the hub's
        // words naming the unit: shown as the unit having lost its lock
        static const QRegularExpression lost(
            QStringLiteral("its time base (was disturbed|is [a-z_]+|entry is gone|changed)|timeline moved against its outputs|\\(disturbance\\)"));
        QString key = str(issue, "key");
        if ((key == QLatin1String("measure.system.abort.hub") || key == QLatin1String("measure.system.abort.disconnected"))
            && lost.match(str(issue, "message")).hasMatch())
            key = QStringLiteral("measure.system.abort.disturbance");
        QJsonObject shown = issue;
        shown.insert(QStringLiteral("key"), key);
        m_issueKey = key;
        m_issueText = issueWords(shown);
    } else if (st.value(QLatin1String("active")).toBool()) {
        m_issueText.clear();
        m_issueKey.clear();
    }
    emit stateChanged();
    if (st.value(QLatin1String("active")).toBool() || !was) {
        updateRunning();
        return;
    }
    endSession();
    // -- WP MDQT2c -- on a device that is not shown: how it ended
    if (MeasureSession *m = measure(); m && !m->isActive())
        m->owner()->report(m, m_issueText.isEmpty() ? tr("The measurement across units ended.") : m_issueText, !m_issueText.isEmpty());
    if (str(st, "phase") != QLatin1String("done")) {
        const QString p = purpose();
        emit stepDone(p == QLatin1String("verify") ? QStringLiteral("verify") : p == QLatin1String("sum_check") ? QStringLiteral("sumCheck") : QStringLiteral("run"), false);
        return;
    }
    if (purpose() == QLatin1String("measure")) loadResult(false);
    else {
        Calls calls = m_calls;
        const QString p = purpose();
        onLoop([calls](FfiCore *c) {
            QJsonArray both{ object(calls(c, QStringLiteral("proposal"), {}, {})), object(calls(c, QStringLiteral("applied"), {}, {})) };
            return compact(both);
        },
               [this, p](const QByteArray &r) {
                   const QJsonArray both = QJsonDocument::fromJson(r).array();
                   if (isOk(both.at(0).toObject())) setProposal(both.at(0).toObject());
                   if (isOk(both.at(1).toObject())) setApplied(both.at(1).toObject());
                   if (p == QLatin1String("verify") && verifyFailed()) say(str(m_applied, "message"));
                   emit stepDone(p == QLatin1String("verify") ? QStringLiteral("verify") : QStringLiteral("sumCheck"), true);
               });
    }
}

void UnitsMeasureController::updateRunning() {
    emit stateChanged();
    if (MeasureSession *m = measure()) {
        const bool on = running();
        if (!on && !m_starting) m_agentLabel.clear();   // -- WP MCP5 --
        QString text = on ? (phaseText().isEmpty() ? tr("Measuring across units.") : phaseText()) : QString();
        //: %1 is "AI · " and the agent's name, %2 what plays
        if (on && !m_agentLabel.isEmpty()) text = tr("Requested by %1: %2").arg(m_agentLabel, text);   // -- WP MCP5 --
        m->setSystemRun(on, text, [this] { stop(); });
    }
}

void UnitsMeasureController::loadResult(bool quiet) {
    Calls calls = m_calls;
    onLoop([calls](FfiCore *c) {
        QJsonArray both{ object(calls(c, QStringLiteral("result"), {}, {})), object(calls(c, QStringLiteral("proposal"), {}, {})) };
        return compact(both);
    },
           [this, quiet](const QByteArray &r) {
               const QJsonArray both = QJsonDocument::fromJson(r).array();
               const QJsonObject res = both.at(0).toObject();
               if (!isOk(res)) {
                   if (!quiet) fail(res, QStringLiteral("run"));
                   return;
               }
               setResult(res);
               if (isOk(both.at(1).toObject())) setProposal(both.at(1).toObject());
               emit stepDone(QStringLiteral("run"), true);
           });
}

void UnitsMeasureController::sumCheck() { runs(QStringLiteral("sumCheck"), QStringLiteral("sumCheck")); }

void UnitsMeasureController::verify() {
    // A new verify asks again (the core's rule)
    disarm();
    runs(QStringLiteral("verify"), QStringLiteral("verify"));
}

// The summed check or the verify: fresh checks, the microphone, the runs
void UnitsMeasureController::runs(const QString &step, const QString &op) {
    clearWords();
    MeasureSession *m = measure();
    if (m_measured.isEmpty() || !m) {
        failWords(tr("Measure the group first."));
        emit stepDone(step, false);
        return;
    }
    if (!placementProblem().isEmpty()) {
        failWords(placementProblem(), QStringLiteral("measure_system_placement"));
        emit stepDone(step, false);
        return;
    }
    if (running() || m->running()) {
        failWords(tr("Stop what plays first."));
        emit stepDone(step, false);
        return;
    }
    const quint64 epoch = m_epoch;
    m_starting = true;
    updateRunning();
    setBusy(op == QLatin1String("sumCheck") ? tr("Starting the summed check") : tr("Starting the verify"));
    const QByteArray setup = m_measured;
    openMicForRun([this, m, epoch, step, op, setup](bool ok, int) {
        if (!ok || epoch != m_epoch) {
            m_starting = false;
            setBusy(QString());
            m->closeMic();
            updateRunning();
            if (!ok && m_error.isEmpty()) failWords(m->error().isEmpty() ? tr("The microphone could not be opened.") : m->error());
            emit stepDone(step, false);
            return;
        }
        m_micOurs = true;
        Calls calls = m_calls;
        withChecks(step, setup, [calls, op](FfiCore *c, const QByteArray &checks) { return calls(c, op, checks, {}); },
                   [this, m, epoch, step, op](const QJsonObject &st) {
                       m_starting = false;
                       setBusy(QString());
                       if (!isOk(st)) {
                           m_micOurs = false;
                           m->closeMic();
                           updateRunning();
                           fail(st, step);
                           // -- review AT-D2 -- a verify refused because the hub's guard has ended: the
                           // core then has the hub's outcome, shown instead of Verify and Keep
                           if (op == QLatin1String("verify")) {
                               Calls calls = m_calls;
                               onLoop([calls](FfiCore *c) { return calls(c, QStringLiteral("applied"), {}, {}); },
                                      [this](const QByteArray &r) {
                                          if (isOk(object(r))) setApplied(object(r));
                                      });
                           }
                           return;
                       }
                       beginSession(st);
                       if (epoch != m_epoch) stop();
                   });
    });
}

// ---- The run in words -------------------------------------------------------------------

QString UnitsMeasureController::phaseText() const {
    const QString ph = phase();
    if (m_starting && (ph == QLatin1String("idle") || !m_state.value(QLatin1String("active")).toBool())) return m_busy;
    if (ph == QLatin1String("idle")) return {};
    const QString p = purpose();
    const QString what = p == QLatin1String("sum_check") ? tr("The summed check") : p == QLatin1String("verify") ? tr("Verifying") : tr("Measuring");
    const int runs = m_state.value(QLatin1String("runs")).toInt();
    const QString run = runs > 1 ? tr(" (run %1 of %2)").arg(std::max(1, m_state.value(QLatin1String("run")).toInt())).arg(runs) : QString();
    const QString words = str(m_state, "words");
    const QString wait = words.isEmpty() ? QString() : tr(": %1").arg(words);
    if (ph == QLatin1String("pre_roll")) return tr("%1: listening to the room first, nothing plays%2").arg(what, wait);
    if (ph == QLatin1String("ramp")) {
        const QJsonValue out = m_state.value(QLatin1String("output"));
        const QJsonValue lvl = m_state.value(QLatin1String("levelDb"));
        const QString serial = str(m_state, "serial").isEmpty() ? tr("a unit") : str(m_state, "serial");
        QString s = tr("%1: finding the level of %2 output %3").arg(what, serial).arg(out.toInt() + 1);
        if (lvl.isDouble()) s += tr(" (%1 dBFS)").arg(lvl.toDouble(), 0, 'f', 1);
        return s + wait;
    }
    if (ph == QLatin1String("run")) return tr("%1: every unit's generator plays on the shared time base%2%3").arg(what, run, wait);
    if (ph == QLatin1String("analysing")) return tr("%1: analysing%2").arg(what, run);
    if (ph == QLatin1String("done")) return tr("%1: finished.").arg(what);
    if (ph == QLatin1String("aborted")) return tr("%1: stopped.").arg(what);
    if (ph == QLatin1String("failed")) return tr("%1: could not finish.").arg(what);
    return what;
}

int UnitsMeasureController::progress() const {
    const double p = m_state.value(QLatin1String("progress")).toDouble();
    return std::isfinite(p) ? int(std::lround(std::clamp(p, 0.0, 1.0) * 100)) : 0;
}

// "Needs N runs": a group that does not fit one capture is chained through the anchor
QString UnitsMeasureController::runsText() const {
    const int runs = m_state.value(QLatin1String("runs")).toInt();
    if (!running() || runs < 2) return {};
    auto *n = Registrar::instance<NoticeCenter>();
    const QVariantMap row = n ? n->row(QStringLiteral("measure_system_runs")) : QVariantMap();
    const QString title = row.value(QStringLiteral("title")).toString(), action = row.value(QStringLiteral("action")).toString();
    return tr("%1: this group needs %2 runs, chained through the anchor output. %3")
        .arg(title.isEmpty() ? tr("The group needs several runs") : title)
        .arg(runs)
        .arg(action.isEmpty() ? tr("Keep the microphone and the room still until every run has finished.") : action);
}

// ---- The result ------------------------------------------------------------------------

void UnitsMeasureController::setResult(const QJsonObject &r) {
    m_result = r;
    m_arrivals.clear();
    m_delays.clear();
    for (const QJsonValue &v : r.value(QLatin1String("arrivals")).toArray()) {
        const QJsonObject a = v.toObject();
        m_arrivals.append(QVariantMap{ { QStringLiteral("label"), outputLabel(str(a, "serial"), a.value(QLatin1String("output")).toInt()) },
                                       { QStringLiteral("arrival"), arrivalWords(a) },
                                       { QStringLiteral("offered"), a.value(QLatin1String("offered")).toBool()
                                                                       ? tr("Yes")
                                                                       : (str(a, "reason").isEmpty() ? tr("No") : str(a, "reason")) } });
    }
    for (const QJsonValue &v : r.value(QLatin1String("delays")).toArray()) {
        const QJsonObject d = v.toObject();
        QString delay = usText(num(d.value(QLatin1String("delayUs"))));
        if (d.value(QLatin1String("uncertaintyUs")).isDouble()) delay += tr(" ± %1").arg(usText(d.value(QLatin1String("uncertaintyUs")).toDouble()));
        m_delays.append(QVariantMap{ { QStringLiteral("from"), outputLabel(str(d, "fromSerial"), d.value(QLatin1String("fromOutput")).toInt()) },
                                     { QStringLiteral("to"), outputLabel(str(d, "toSerial"), d.value(QLatin1String("toOutput")).toInt()) },
                                     { QStringLiteral("delay"), delay },
                                     { QStringLiteral("offered"), d.value(QLatin1String("offered")).toBool()
                                                                     ? tr("Yes")
                                                                     : (str(d, "reason").isEmpty() ? tr("No") : str(d, "reason")) } });
    }
    m_graph.clear();
    emit resultChanged();
    requestCurves();
}

QString UnitsMeasureController::timingText() const {
    if (!hasResult()) return {};
    QString s;
    if (timingReliable()) s = tr("The timing of every run was reliable.");
    else {
        const QJsonArray reasons = m_result.value(QLatin1String("reasons")).toArray();
        s = issueWords(QJsonObject{ { QStringLiteral("key"), QStringLiteral("measure.system.timing_unreliable") },
                                    { QStringLiteral("message"), reasons.isEmpty() ? tr("the capture clock wandered between the units' sweeps")
                                                                                   : reasons.at(0).toString() } });
    }
    const int runs = m_result.value(QLatin1String("runs")).toInt();
    if (runs > 1) s += tr(" %1 runs, chained through the anchor output.").arg(runs);
    return s;
}

QStringList UnitsMeasureController::reasons() const {
    QStringList out;
    for (const QJsonValue &v : m_result.value(QLatin1String("reasons")).toArray()) out << v.toString();
    return out;
}

// Each unit's document's curves from the core (dspi_measure_doc_curves),
// drawn when they come; only the last ask's answer is drawn
void UnitsMeasureController::requestCurves() {
    const quint64 ask = ++m_curvesAsk;
    QList<QPair<QString, QByteArray>> docs;
    for (const QJsonValue &v : m_result.value(QLatin1String("documents")).toArray())
        docs.append({ str(v.toObject(), "serial"), str(v.toObject(), "document").toUtf8() });
    if (docs.isEmpty()) return;
    m_ctx.core->worker()->call(this,
                               [docs](FfiCore *) {
                                   QJsonArray all;
                                   for (const auto &[serial, doc] : docs) {
                                       QJsonObject c = object(MeasureController::reply([&](char *b, uint32_t l) {
                                           return dspi_measure_doc_curves(doc.constData(), R"({"smoothing":6})", b, l);
                                       }, 1 << 20));
                                       c.insert(QStringLiteral("serial"), serial);
                                       all.append(c);
                                   }
                                   return compact(all);
                               },
                               [this, ask](QByteArray r) {
                                   if (ask != m_curvesAsk) return;
                                   QVariantList series;
                                   bool spl = false;
                                   int colour = 0;
                                   for (const QJsonValue &u : QJsonDocument::fromJson(r).array()) {
                                       const QJsonObject c = u.toObject();
                                       if (!isOk(c)) continue;
                                       for (const QJsonValue &o : c.value(QLatin1String("outputs")).toArray()) {
                                           const QJsonObject out = o.toObject();
                                           const QJsonObject measured = out.value(QLatin1String("measured")).toObject();
                                           const int col = colour++;
                                           if (measured.isEmpty()) continue;
                                           spl = spl || measured.value(QLatin1String("spl")).toBool();
                                           series.append(MeasureController::series(
                                               measured, QStringLiteral("measured"), outputLabel(str(c, "serial"), out.value(QLatin1String("output")).toInt()), col));
                                       }
                                   }
                                   m_graph = series.isEmpty() ? QVariantMap() : MeasureController::graph(series, spl ? tr("dB SPL") : tr("dB (relative)"));
                                   emit resultChanged();
                               });
}

// ---- The proposal ----------------------------------------------------------------------

void UnitsMeasureController::setProposal(const QJsonObject &p) {
    m_proposal = p;
    emit proposalChanged();
}

QStringList UnitsMeasureController::proposalArrivals() const {
    QStringList out;
    for (const QJsonValue &v : m_proposal.value(QLatin1String("arrivals")).toArray()) {
        const QJsonObject a = v.toObject();
        out << tr("%1: %2").arg(outputLabel(str(a, "serial"), a.value(QLatin1String("output")).toInt()), arrivalWords(a));
    }
    return out;
}

QStringList UnitsMeasureController::withheld() const {
    QStringList out;
    for (const QJsonValue &v : m_proposal.value(QLatin1String("withheld")).toArray()) {
        const QJsonObject a = v.toObject();
        out << tr("%1: %2").arg(outputLabel(str(a, "serial"), a.value(QLatin1String("output")).toInt()),
                                str(a, "reason").isEmpty() ? tr("not offered") : str(a, "reason"));
    }
    return out;
}

QVariantList UnitsMeasureController::transitions() const {
    QVariantList out;
    for (const QJsonValue &v : m_proposal.value(QLatin1String("transitions")).toArray()) {
        const QJsonObject t = v.toObject();
        const bool checked = t.value(QLatin1String("checked")).toBool(), passed = t.value(QLatin1String("passed")).toBool();
        const QString verdict = !str(t, "message").isEmpty() ? str(t, "message")
                                : checked ? (passed ? tr("the summed check agreed") : tr("the summed check did not agree"))
                                          : tr("not checked yet");
        out.append(QVariantMap{
            { QStringLiteral("text"), tr("%1 with %2, %3 to %4 Hz: %5")
                                          .arg(outputLabel(str(t, "aSerial"), t.value(QLatin1String("aOutput")).toInt()),
                                               outputLabel(str(t, "bSerial"), t.value(QLatin1String("bOutput")).toInt()))
                                          .arg(qRound(t.value(QLatin1String("bandLoHz")).toDouble()))
                                          .arg(qRound(t.value(QLatin1String("bandHiHz")).toDouble()))
                                          .arg(verdict) },
            { QStringLiteral("failed"), checked && !passed } });
    }
    return out;
}

// -- WP ATQTSIM --
QString UnitsMeasureController::guardNote() const { return sentence(m_proposal.value(QLatin1String("guardNote")).toString()); }

QString UnitsMeasureController::proposalNotice() const { return issueWords(m_proposal.value(QLatin1String("notice")).toObject()); }

bool UnitsMeasureController::mayApply() const {
    return hasProposal() && m_proposal.value(QLatin1String("offered")).toBool() && !sumCheckNeeded()
           && m_proposal.value(QLatin1String("mayApply")).toBool() && admin();
}

QString UnitsMeasureController::applyWhyKey() const {
    if (!hasProposal() || mayApply()) return {};
    if (!m_proposal.value(QLatin1String("offered")).toBool()) return str(m_proposal.value(QLatin1String("notice")).toObject(), "key");
    if (sumCheckNeeded()) return QStringLiteral("measure.system.sum_check_needed");
    return QStringLiteral("measure.system.admin");
}

QString UnitsMeasureController::applyWhy() const {
    if (!hasProposal() || mayApply()) return {};
    if (!m_proposal.value(QLatin1String("offered")).toBool())
        return proposalNotice().isEmpty() ? tr("Nothing can be applied from this measurement.") : proposalNotice();
    if (sumCheckNeeded())
        return issueWords(QJsonObject{ { QStringLiteral("key"), QStringLiteral("measure.system.sum_check_needed") },
                                       { QStringLiteral("message"), tr("run the summed check first") } });
    auto *n = Registrar::instance<NoticeCenter>();
    const QVariantMap row = n ? n->row(QStringLiteral("measure.system.admin")) : QVariantMap();
    if (!row.isEmpty()) return tr("%1: %2").arg(row.value(QStringLiteral("title")).toString(), row.value(QStringLiteral("reason")).toString());
    return tr("Applying an alignment across units needs the Admin role.");
}

void UnitsMeasureController::apply() {
    clearWords();
    if (!hasProposal()) return;
    // Admin's only: nothing is sent for another role
    if (!admin()) {
        failWords(tr("Applying an alignment across units needs the Admin role."), QStringLiteral("measure.system.admin"));
        emit stepDone(QStringLiteral("apply"), false);
        return;
    }
    if (running()) return;
    disarm();
    setBusy(tr("Applying"));
    const QByteArray id = QByteArray::number(m_proposal.value(QLatin1String("id")).toInt());
    Calls calls = m_calls;
    onLoop([calls, id](FfiCore *c) { return calls(c, QStringLiteral("apply"), id, {}); },
           [this](const QByteArray &r) {
               setBusy(QString());
               const QJsonObject a = object(r);
               if (!isOk(a)) {
                   fail(a, QStringLiteral("apply"));
                   return;
               }
               setApplied(a);
               // The hub's guard counts from now
               m_guardS = a.value(QLatin1String("guardS")).toInt();
               m_guardSince.start();
               if (m_guardS > 0) m_guardTimer.start();
               emit guardChanged();
               say(str(a, "message").isEmpty() ? tr("Applied: measure the summed runs again to verify it, then Keep or Revert.") : str(a, "message"));
               emit stepDone(QStringLiteral("apply"), true);
           });
}

// ---- After the apply -------------------------------------------------------------------

// -- WP AIVIS -- An agent's step across units: what is proposed and applied
void UnitsMeasureController::readAgain() {
    if (m_shutDown || !m_entered || running() || !m_busy.isEmpty()) return;
    Calls calls = m_calls;
    onLoop([calls](FfiCore *c) {
        QJsonArray both{ object(calls(c, QStringLiteral("applied"), {}, {})), object(calls(c, QStringLiteral("proposal"), {}, {})) };
        return compact(both);
    },
           [this](const QByteArray &r) {
               const QJsonArray both = QJsonDocument::fromJson(r).array();
               if (isOk(both.at(0).toObject())) setApplied(both.at(0).toObject());
               if (isOk(both.at(1).toObject())) setProposal(both.at(1).toObject());
               // Applied by the agent a moment ago: the guard's countdown runs here too
               const QString s = stage();
               const int guard = m_applied.value(QLatin1String("guardS")).toInt();
               if (m_guardS == 0 && guard > 0 && (s == QLatin1String("applied") || s == QLatin1String("verified"))) {
                   m_guardS = guard;
                   m_guardSince.start();
                   m_guardTimer.start();
                   emit guardChanged();
               }
           });
}

void UnitsMeasureController::setApplied(const QJsonObject &a) {
    m_applied = a;
    const QString s = stage();
    // The guard ends with a Keep or a Revert (or the hub's own revert)
    if (s == QLatin1String("kept") || s == QLatin1String("reverted") || s == QLatin1String("revert_incomplete") // -- WP REVERTW --
        || s == QLatin1String("none")) {
        m_guardS = 0;
        m_guardTimer.stop();
        emit guardChanged();
        if (m_question) disarm();
    }
    emit appliedChanged();
}

bool UnitsMeasureController::verifyFailed() const {
    return stage() == QLatin1String("verified") && m_applied.value(QLatin1String("verified")).isBool()
           && !m_applied.value(QLatin1String("verified")).toBool();
}

QString UnitsMeasureController::appliedText() const {
    const QString s = stage();
    if (s == QLatin1String("applied")) return tr("Applied: the summed runs are measured again with the new delays to verify it.");
    if (s == QLatin1String("verified")) {
        if (!verifyFailed()) return tr("Verified: the summed runs agree with the new delays.");
        return issueWords(QJsonObject{ { QStringLiteral("key"), QStringLiteral("measure.system.verify_failed") },
                                       { QStringLiteral("message"), str(m_applied, "message").isEmpty()
                                                                        ? tr("the summed runs did not agree with the new delays")
                                                                        : str(m_applied, "message") } });
    }
    if (s == QLatin1String("kept")) return tr("Kept.");
    if (s == QLatin1String("reverted")) {
        // The hub's words when its guard put it back (WP ATGUARD)
        const QString msg = str(m_applied, "message");
        // -- review AT-D2 -- the core's words as they are (its own begin "reverted: ..."; the hub's say
        // when it could NOT put everything back, which "Reverted:" before them would deny)
        if (msg.isEmpty()) return tr("Reverted: the installation and every unit are as they were filed.");
        return msg.at(0).toUpper() + msg.mid(1);
    }
    // -- WP REVERTW -- not fully put back: the notice's title, what was not (the core's words), what to do
    if (s == QLatin1String("revert_incomplete")) {
        QString key = str(m_applied, "revertNotice");
        if (key.isEmpty()) key = QStringLiteral("revert.unknown");
        if (auto *notices = Registrar::instance<NoticeCenter>()) return notices->revertText(key, str(m_applied, "message"));
        return tr("The revert was not fully successful. %1 Check the settings, or restore a backup (a snapshot).").arg(str(m_applied, "message"));
    }
    return {};
}

int UnitsMeasureController::guardRemainingS() const {
    if (m_guardS <= 0 || !m_guardSince.isValid()) return 0;
    return std::max(0, m_guardS - int(m_guardSince.elapsed() / 1000));
}

QString UnitsMeasureController::guardText() const {
    const QString s = stage();
    if (m_guardS <= 0 || (s != QLatin1String("applied") && s != QLatin1String("verified"))) return {};
    const int left = guardRemainingS();
    if (left <= 0)
        return tr("The hub's guard has run out: unless it was kept, the hub has put the installation back by itself. Keep shows what the hub did.");
    return tr("Keep or Revert within %n s: without Keep the hub puts everything back by itself (also if this console's "
              "connection is lost).", nullptr, left);
}

int UnitsMeasureController::keepWaitS() const {
    if (!m_question || !m_refusedAt.isValid()) return 0;
    return std::max(0, int(std::ceil((kConfirmMs - m_refusedAt.elapsed()) / 1000.0)));
}

void UnitsMeasureController::keep() {
    // After a refusal the person answers the question first, every time
    if (m_question) return;
    keepNow();
}

void UnitsMeasureController::keepNow() {
    clearWords();
    setBusy(tr("Keeping"));
    Calls calls = m_calls;
    onLoop([calls](FfiCore *c) {
        QJsonArray both{ object(calls(c, QStringLiteral("keep"), {}, {})), object(calls(c, QStringLiteral("applied"), {}, {})) };
        return compact(both);
    },
           [this](const QByteArray &r) {
               setBusy(QString());
               const QJsonArray both = QJsonDocument::fromJson(r).array();
               keepAnswered(both.at(0).toObject(), both.at(1).toObject());
           });
}

void UnitsMeasureController::keepAnswered(const QJsonObject &reply, const QJsonObject &applied) {
    if (isOk(reply)) {
        disarm();
        setApplied(reply);
        // Kept, or reverted already by the hub's guard (its words)
        say(appliedText());
        emit stepDone(QStringLiteral("keep"), stage() == QLatin1String("kept"));
        return;
    }
    if (isOk(applied)) setApplied(applied);
    if (verifyFailed()) {
        // The core's refusal is the question: only the person's second press keeps
        ++m_keepRefusals;
        m_refusedAt.start();
        m_question = true;
        // -- WP ATQTSIM -- the core's words as a sentence
        m_questionText = str(m_applied, "message").isEmpty() ? MeasureController::failureText(reply) : sentence(str(m_applied, "message"));
        m_keepTimer.start();
        emit keepChanged();
        emit stepDone(QStringLiteral("keep"), false);
        return;
    }
    fail(reply, QStringLiteral("keep"));
}

void UnitsMeasureController::confirmKeep() {
    // Only the person's answer to the open question
    if (!m_question) return;
    m_question = false;
    m_keepTimer.stop();
    emit keepChanged();
    // An answer within the core's second is sent when it counts, unless
    // something withdrew it meanwhile (Revert, Stop, a new apply or verify)
    const qint64 wait = m_refusedAt.isValid() ? kConfirmMs + 200 - m_refusedAt.elapsed() : 0;
    if (wait > 0) {
        const int token = ++m_keepToken;
        QTimer::singleShot(int(wait), this, [this, token] {
            if (token == m_keepToken) keepNow();
        });
    } else keepNow();
}

void UnitsMeasureController::declineKeep() {
    if (!m_question) return;
    m_question = false;
    m_keepTimer.stop();
    emit keepChanged();
}

void UnitsMeasureController::disarm() {
    ++m_keepToken;
    if (!m_question) return;
    m_question = false;
    m_questionText.clear();
    m_keepTimer.stop();
    emit keepChanged();
}

void UnitsMeasureController::revert() {
    clearWords();
    // A delayed Keep is withdrawn: Revert is the last word
    disarm();
    if (!admin()) {
        failWords(tr("Reverting an alignment across units needs the Admin role."), QStringLiteral("measure.system.admin"));
        emit stepDone(QStringLiteral("revert"), false);
        return;
    }
    setBusy(tr("Reverting"));
    Calls calls = m_calls;
    // -- WP REVERTW -- and where it stands after: a refused revert may have ended the guard with
    // nothing set back (its own stage)
    onLoop([calls](FfiCore *c) {
        QJsonArray both{ object(calls(c, QStringLiteral("revert"), {}, {})), object(calls(c, QStringLiteral("applied"), {}, {})) };
        return compact(both);
    },
           [this](const QByteArray &r) {
               setBusy(QString());
               const QJsonArray both = QJsonDocument::fromJson(r).array();
               const QJsonObject a = both.at(0).toObject();
               if (!isOk(a)) {
                   if (isOk(both.at(1).toObject())) setApplied(both.at(1).toObject());
                   fail(a, QStringLiteral("revert"));
                   return;
               }
               setApplied(a);
               say(appliedText());
               emit stepDone(QStringLiteral("revert"), true);
           });
}

// -- WP MCP5 -- An AI agent's measurement across units, its summed check or its
// verify: the checks were the agent's tool's (each unit's roles from its own
// confirmed speaker layout); here the microphone, then dspi_mcp_measure_begin on
// the device shown, driven as this view's own session.
void UnitsMeasureController::agentRequest(const QJsonObject &ticket) {
    if (ticket.value(QLatin1String("loop")).toString() != QLatin1String("system")) return;
    auto *ai = Registrar::instance<AiController>();
    if (!ai) return;
    const quint64 id = quint64(ticket.value(QLatin1String("request_id")).toDouble());
    QPointer<AiController> back(ai);
    const auto done = [this, back, id](bool started, const QString &note) {
        QPointer<UnitsMeasureController> self(this);
        if (back) back->reportMeasure(id, started, note, [self] {   // -- WP RCFIX --
            if (self) self->stop();
        });
    };
    MeasureSession *m = measure();
    if (!m || m_shutDown) {
        done(false, tr("The Measure view is not ready."));
        return;
    }
    if (ticket.value(QLatin1String("device")).toString() != m_ctx.registry->activeId() || !m->isActive()) {
        done(false, tr("Measuring across units runs on the device the app shows: ask the user to show that device."));
        return;
    }
    if (running() || m_starting || m->running()) {
        done(false, tr("Something already runs on this device in the app; nothing was played."));
        return;
    }
    if (!m->playBlocker().isEmpty()) {
        done(false, m->playBlocker());
        return;
    }
    // -- WP LAY7 -- (INT5 follow-up 3) on its own tab
    if (auto *mc = Registrar::instance<MeasureController>()) mc->showPart(QStringLiteral("units"));
    clearWords();
    const QString kind = ticket.value(QLatin1String("kind")).toString();
    const QByteArray setup = QJsonDocument(ticket.value(QLatin1String("setup")).toObject().value(QLatin1String("setup")).toObject())
                                 .toJson(QJsonDocument::Compact);
    const QByteArray text = QJsonDocument(ticket).toJson(QJsonDocument::Compact);
    const quint64 epoch = m_epoch;
    m_starting = true;
    m_agentLabel = ticket.value(QLatin1String("client_name")).toString();
    updateRunning();
    setBusy(tr("Starting"));
    openMicForRun([this, m, epoch, done, text, kind, setup](bool ok, int rateHz) {
        if (!ok || epoch != m_epoch) {
            m_starting = false;
            setBusy(QString());
            m->closeMic();
            updateRunning();
            done(false, ok ? tr("Stopped in the app before it began.")
                           : (m->error().isEmpty() ? tr("The microphone could not be opened.") : m->error()));
            return;
        }
        m_micOurs = true;
        onLoop([text, rateHz](FfiCore *c) {
                   return once([&](char *b, uint32_t l) { return dspi_mcp_measure_begin(c, text.constData(), uint32_t(rateHz), b, l); });
               },
               [this, m, epoch, done, kind, setup](const QByteArray &r) {
                   m_starting = false;
                   setBusy(QString());
                   const QJsonObject st = object(r);
                   if (!isOk(st)) {
                       m_micOurs = false;
                       m->closeMic();
                       updateRunning();
                       fail(st, QStringLiteral("start"));
                       done(false, str(st, "error"));
                       return;
                   }
                   if (kind == QLatin1String("system")) {
                       m_measured = setup;
                       m_result = {};
                       m_arrivals.clear();
                       m_delays.clear();
                       m_graph.clear();
                       emit resultChanged();
                       setProposal({});
                       setApplied({});
                       disarm();
                   }
                   beginSession(st);
                   done(true, QString());
                   if (epoch != m_epoch) stop();
               });
    });
}

