// -- WP XDEV1 -- Views that span devices, on the registry (MPLAN 1.2 item 6):
// Several units measures a unit open in a tab on that tab's core (no second
// session), and a unit that is not on an extra core of the set that holds its
// serial and closes with the tab; "Open in tab" from the System view; and
// review RB's folds: a write refused on a device that is not the active one is
// a notice naming it (B4), and leaving Modern keeps what is open (F2).
//
// Against the real hub `uconsole --demo hub --alink --with-measure-units`
// (DSPI_SMOKE_UCONSOLE), as atqt-units; the run uses the demo hub's simulated
// microphone, so nothing is opened here. Section `xdev1-devices` (975).
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "LinkController.h"
#include "MeasureController.h"
#include "ModeController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "Shell.h"
#include "SystemController.h"
#include "UnitsMeasureController.h"

#include "CoreApi.h"

#include <QDir>
#include <QElapsedTimer>
#include <QHash>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMutex>
#include <QPointer>
#include <QProcess>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QThread>
#include <cstdio>
#include <functional>

namespace {

const QString A = QStringLiteral("DEMO-TB-A");
const QString B = QStringLiteral("DEMO-TB-B");
const QString C = QStringLiteral("DEMO-TB-C");

// One `uconsole --demo hub --alink --with-measure-units` (SmokeMeasureUnits.cpp's)
class MeasureHub {
public:
    bool start(const QString &store) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_UCONSOLE");
        if (binary.isEmpty()) return false;
        QDir(store).removeRecursively();
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList{ "--demo", "hub", "--alink", "--with-measure-units", "--loopback", "--json", "--no-keep-awake",
                                          "--open", "--forget-clients", "--store", store, "--port", "0" });
        if (!m_proc.waitForStarted(5000)) return false;
        QElapsedTimer t;
        t.start();
        while (t.elapsed() < 20000) {
            if (!m_proc.canReadLine() && !m_proc.waitForReadyRead(1000)) {
                if (m_proc.state() != QProcess::Running) return false;
                continue;
            }
            while (m_proc.canReadLine()) {
                const QJsonObject e = QJsonDocument::fromJson(m_proc.readLine()).object();
                if (e.value(QLatin1String("event")).toString() == QLatin1String("status")) port = e.value(QLatin1String("port")).toInt();
            }
            if (port > 0) {
                url = QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(port);
                return true;
            }
        }
        return false;
    }
    void stop() {
        if (m_proc.state() == QProcess::NotRunning) return;
        m_proc.kill();
        m_proc.waitForFinished(5000);
    }
    ~MeasureHub() { stop(); }
    int port = 0;
    QString url;

private:
    QProcess m_proc;
};

bool isOkJson(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }

QJsonObject cjson(const std::function<uint32_t(char *, uint32_t)> &f) {
    QByteArray buf(256 * 1024, '\0');
    const uint32_t n = f(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    return QJsonDocument::fromJson(QByteArray(buf.constData(), int(n) - 1)).object();
}

// A unit's volume lowered through a Link client of the smoke's own: S2 refuses
// a measurement at the demo's volume (SmokeMeasureUnits.cpp's)
bool lowerVolume(const QString &url, const QString &serial) {
    FfiCore *c = dspi_core_new();
    if (!c) return false;
    const QByteArray u = url.toUtf8(), s = serial.toUtf8();
    bool ok = isOkJson(cjson([&](char *b, uint32_t l) { return dspi_link_connect(c, u.constData(), "xdev1-smoke", "0", "", "", b, l); }));
    int handle = -1;
    for (int i = 0; ok && handle < 0 && i < 50; ++i) {
        for (const QJsonValue &d : cjson([&](char *b, uint32_t l) { return dspi_link_devices(c, b, l); }).value(QLatin1String("value")).toArray())
            if (d.toObject().value(QLatin1String("serial")).toString() == serial) handle = d.toObject().value(QLatin1String("handle")).toInt(-1);
        if (handle < 0) QThread::msleep(100);
    }
    ok = ok && handle >= 0
         && isOkJson(cjson([&](char *b, uint32_t l) { return dspi_link_select_device(c, uint32_t(handle), s.constData(), b, l); }))
         && dspi_fetch_all(c) && dspi_set_user_volume(c, -20.0f);
    dspi_link_close(c);
    dspi_core_free(c);
    return ok;
}

// The unit checks the controller asked, with the core each ran on
struct Record {
    QMutex lock;
    QHash<QString, QList<FfiCore *>> checks;   // serial -> cores
    void add(const QString &serial, FfiCore *c) {
        QMutexLocker l(&lock);
        checks[serial].append(c);
    }
    QList<FfiCore *> of(const QString &serial) {
        QMutexLocker l(&lock);
        return checks.value(serial);
    }
    void clear() {
        QMutexLocker l(&lock);
        checks.clear();
    }
};

bool allAre(const QList<FfiCore *> &cores, FfiCore *c) {
    if (cores.isEmpty() || !c) return false;
    for (FfiCore *x : cores)
        if (x != c) return false;
    return true;
}

struct ScopeExit {
    std::function<void()> f;
    ~ScopeExit() {
        if (f) f();
    }
};

// ---- B4 and F2, on two demos ----------------------------------------------------------------

void demosPart(SmokeDriver &d, DeviceRegistry *r, NoticeCenter *notices) {
    if (!d.app().device->connected() || !d.app().device->demo() || d.app().device->remote()) d.app().device->openDemo();
    d.waitIdle();
    DeviceSession *a = r->mainSession();
    r->setKeepOpen(true);
    r->openDemo();
    d.check("xdev1: a second demo opens beside the first",
            d.waitFor(10000, [&] { return r->opening() == 0 && r->sessions().size() == 2; }));
    if (r->sessions().size() != 2) {
        r->setKeepOpen(false);
        return;
    }
    QPointer<DeviceSession> b = r->sessions().at(1);
    d.waitIdle();

    // B4: a refusal on the device left is a notice that names it; the active one's as before
    r->activate(a);
    notices->clear();
    emit b->model()->writeRefused(QStringLiteral("xdev1 gain"), QStringLiteral("limit_exceeded"), QStringLiteral("above the limit"));
    d.spin(100);
    const QString shown = notices->property("current").toMap().value(QStringLiteral("text")).toString();
    std::printf("smoke: xdev1: background refusal: %s\n", qPrintable(shown));
    d.check("xdev1: a write refused on a device that is not the active one is a notice naming it (RB B4)",
            shown.startsWith(QStringLiteral("On %1: ").arg(b->title())));
    notices->clear();
    const int before = notices->serial();
    emit b->model()->writeRefused(QStringLiteral("xdev1 gain"), QStringLiteral("not_a_gate"), QStringLiteral("x"));
    emit a->model()->writeRefused(QStringLiteral("xdev1 gain"), QStringLiteral("limit_exceeded"), QStringLiteral("above the limit"));
    d.spin(100);
    const QString own = notices->property("current").toMap().value(QStringLiteral("text")).toString();
    d.check("xdev1: only the refusals the notices show are forwarded, and the active one's is not named",
            notices->serial() == before + 1 && !own.startsWith(QLatin1String("On ")));
    notices->clear();

    // F2: leaving Modern keeps the other device open; back in Modern its tab is there
    d.check("xdev1: the policy is that the others stay open in Classic", DeviceRegistry::kOthersStayOpenInClassic);
    r->setKeepOpen(false);
    d.spin(300);
    d.check("xdev1: leaving Modern leaves the other device open, the active one active (RB F2)",
            b && r->sessions().size() == 2 && r->active() == a && b->model()->connected());
    r->setKeepOpen(true);
    d.spin(200);
    d.check("xdev1: back in Modern both are tabs again", r->devices().size() == 2);
    if (b) r->close(b->id());
    d.waitFor(10000, [&] { return b.isNull(); });
    r->setKeepOpen(false);
    d.waitIdle();
}

// ---- Several units and Open in tab, on the hub ---------------------------------------------

void hubPart(SmokeDriver &d, DeviceRegistry *r, NoticeCenter *notices) {
    auto *link = Registrar::instance<LinkController>();
    auto *mc = Registrar::instance<MeasureController>();
    auto *uc = Registrar::instance<UnitsMeasureController>();
    auto *sys = Registrar::instance<SystemController>();
    auto *modes = Registrar::instance<ModeController>();
    d.check("xdev1: the controllers", link && mc && uc && sys && modes);
    if (!link || !mc || !uc || !sys || !modes) return;
    const QString out = d.outDir() + QStringLiteral("/xdev1");
    QDir().mkpath(out);
    MeasureHub hub;
    const bool up = hub.start(out + QStringLiteral("/hub"));
    d.check("xdev1: uconsole --demo hub --alink --with-measure-units runs (DSPI_SMOKE_UCONSOLE)", up);
    if (!up) return;
    bool lowered = true;
    for (const QString &s : { A, B, C }) lowered = lowerVolume(hub.url, s) && lowered;
    d.check("xdev1: each unit's volume lowered through a Link client of its own (S2)", lowered);

    const QString modeBefore = modes->mode();
    modes->choose(QStringLiteral("advanced"), false);
    DeviceSession *main = r->mainSession();
    QStringList steps;
    QHash<QString, bool> results;
    const QMetaObject::Connection conn = QObject::connect(uc, &UnitsMeasureController::stepDone, uc, [&](const QString &s, bool ok) {
        steps << s;
        results.insert(s, ok);
    });
    QQuickWindow *w = nullptr;
    const ScopeExit cleanup{ [&] {
        QObject::disconnect(conn);
        uc->setCallsForTest(nullptr);
        if (w) w->close();
        mc->setCaptureSource(nullptr);
        for (DeviceSession *s : r->sessions())
            if (s != main) r->close(s->id());
        d.waitFor(15000, [&] { return r->sessions().size() == 1 && r->extraSerials().isEmpty(); });
        r->activate(main);
        r->setKeepOpen(false);
        hub.stop();
        modes->choose(modeBefore, false);
        notices->clear();
        d.waitIdle();
    } };
    const auto step = [&](const QString &s, int ms) { return d.waitFor(ms, [&] { return steps.contains(s); }); };
    const auto in = [&](const QString &name) { return w ? d.item(name, w->contentItem()) : nullptr; };
    QQuickItem *tabs = nullptr;
    const auto showUnits = [&] {
        w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("tools.measure")));
        if (!w) return false;
        d.waitFor(2000, [&] { return w->isVisible(); });
        tabs = in(QStringLiteral("Measure.tabs"));
        if (!tabs) return false;
        tabs->setProperty("currentIndex", 1);
        d.spin(100);
        steps.clear();
        tabs->setProperty("currentIndex", 6);
        return step(QStringLiteral("enter"), 10000);
    };
    const auto leaveUnits = [&] {
        if (tabs) tabs->setProperty("currentIndex", 1);
        d.spin(200);
    };
    const auto sessionOf = [&](const QString &serial) { return r->holder(serial); };

    // ---- A unit of the hub in a tab (Modern): A, through the lobby
    r->setKeepOpen(true);
    link->connectToHub(hub.url);
    const bool listed = d.waitFor(30000, [&] {
        int n = 0;
        for (const QVariant &v : link->devices()) n += QStringList{ A, B, C }.contains(v.toMap().value(QStringLiteral("serial")).toString());
        return !link->busy() && link->phase() == QLatin1String("choose_device") && n == 3;
    });
    for (const QVariant &v : link->devices())
        if (v.toMap().value(QStringLiteral("serial")).toString() == A) link->bindDevice(v.toMap().value(QStringLiteral("handle")).toInt(), A);
    QPointer<DeviceSession> sa;
    const bool bound = listed && d.waitFor(20000, [&] {
        sa = sessionOf(A);
        return r->opening() == 0 && sa && r->active() == sa && sa->model()->connected();
    }) && d.waitIdle(20000);
    d.check("xdev1: DEMO-TB-A opens in a tab of its own, beside the demo", bound && r->sessions().size() == 2);
    if (!bound) return;

    // ---- Open in tab from the System view
    sys->refresh();
    d.check("xdev1: the System view reads A's hub, and Open in tab is offered (Modern)",
            d.waitFor(15000, [&] { return sys->available() && sys->openInTabOffered(); }));
    bool rowB = false;
    for (const QVariant &v : sys->units()) {
        const QVariantMap u = v.toMap();
        rowB = rowB || (u.value(QStringLiteral("serial")).toString() == B && u.value(QStringLiteral("kind")).toString() == QLatin1String("unit"));
    }
    d.check("xdev1: the System view lists DEMO-TB-B as a unit", rowB);
    d.check("xdev1: A's own row is the active tab", sys->tabs().value(A).toString() == QLatin1String("active"));
    QSignalSpy failed(r, &DeviceRegistry::openFailed);
    d.check("xdev1: a serial the hub does not list is not opened", !sys->openInTab(QStringLiteral("DEMO-TB-NONE")) && r->opening() == 0);
    d.check("xdev1: Open in tab for DEMO-TB-B", sys->openInTab(B));
    QPointer<DeviceSession> sb;
    const bool bOpen = d.waitFor(20000, [&] {
        sb = sessionOf(B);
        return r->opening() == 0 && sb && r->active() == sb && sb->model()->connected();
    }) && d.waitIdle(20000);
    d.check("xdev1: DEMO-TB-B opens in a tab of its own and is shown", bOpen && r->sessions().size() == 3 && failed.isEmpty());
    if (!bOpen) return;
    d.check("xdev1: its row says so", d.waitFor(3000, [&] { return sys->tabs().value(B).toString() == QLatin1String("active")
                                                                   && sys->tabs().value(A).toString() == QLatin1String("open"); }));
    r->activate(sa);
    d.waitIdle();
    d.check("xdev1: Open in tab for a unit that is open shows its tab, nothing opens",
            sys->openInTab(B) && r->active() == sb && r->sessions().size() == 3 && r->opening() == 0);
    r->activate(sa);
    d.waitIdle();
    {
        QString idB, idC;
        auto *sw = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("window.system")));
        QQuickItem *showB = nullptr, *openC = nullptr, *ownA = nullptr;
        // The view is read again after the switch
        const bool buttons = sw && d.waitFor(15000, [&] {
            for (const QVariant &v : sys->units()) {
                const QVariantMap u = v.toMap();
                if (u.value(QStringLiteral("serial")).toString() == B) idB = u.value(QStringLiteral("id")).toString();
                if (u.value(QStringLiteral("serial")).toString() == C) idC = u.value(QStringLiteral("id")).toString();
            }
            if (idB.isEmpty() || idC.isEmpty()) return false;
            showB = d.item(QStringLiteral("SystemWindow.openInTab.") + idB, sw->contentItem());
            openC = d.item(QStringLiteral("SystemWindow.openInTab.") + idC, sw->contentItem());
            return showB && openC && showB->isVisible() && openC->isVisible();
        });
        for (const QVariant &v : sys->units())
            if (v.toMap().value(QStringLiteral("serial")).toString() == A)
                ownA = sw ? d.item(QStringLiteral("SystemWindow.openInTab.") + v.toMap().value(QStringLiteral("id")).toString(), sw->contentItem()) : nullptr;
        std::printf("smoke: xdev1: rows: %d B %s %s, C %s %s, A %s %d\n", int(buttons), qPrintable(idB),
                    showB ? qPrintable(showB->property("text").toString()) : "-", qPrintable(idC),
                    openC ? qPrintable(openC->property("text").toString()) : "-", ownA ? "item" : "none", ownA ? int(ownA->isVisible()) : -1);
        d.check("xdev1: the System window's rows: Show tab for B (open), Open in tab for C, none for A (shown)",
                buttons && showB->property("text").toString() == QLatin1String("Show tab")
                    && openC->property("text").toString() == QLatin1String("Open in tab") && (!ownA || !ownA->isVisible()));
        d.grab(sw, QStringLiteral("xdev1/system-open-in-tab.png"));
        if (sw) sw->close();
    }
    // Classic: not offered
    r->setKeepOpen(false);
    d.check("xdev1: Open in tab is not offered in Classic", !sys->openInTabOffered() && !sys->openInTab(C));
    r->setKeepOpen(true);

    // ---- Several units on A: B is in a tab, C is not
    Record rec;
    uc->setCallsForTest([&rec](FfiCore *c, const QString &op, const QByteArray &a, const QByteArray &b) {
        if (op == QLatin1String("unitCheck"))
            rec.add(QJsonDocument::fromJson(b).object().value(QLatin1String("serial")).toString(), c);
        return UnitsMeasureController::coreCall(c, op, a, b);
    });
    const bool shownUnits = showUnits();
    d.check("xdev1: the Several units tab on A", shownUnits && uc->unavailable().isEmpty() && uc->units().size() >= 3);
    if (!shownUnits) return;
    uc->setPlacement(QStringLiteral("listeningPosition"));
    steps.clear();
    uc->setUnitChosen(A, true);
    uc->setUnitChosen(B, true);
    uc->setUnitChosen(C, true);
    const bool opened = d.waitFor(25000, [&] { return uc->unitOpen(B) && uc->unitOpen(C); });
    d.check("xdev1: B and C have their connections", opened);
    d.check("xdev1: B, open in a tab, is judged by that tab's core: no second session",
            uc->unitInTab(B) && uc->unitCore(B) == sb->handle() && r->sessions().size() == 3);
    bool coreOfATab = false;
    for (DeviceSession *s : r->sessions()) coreOfATab = coreOfATab || s->handle() == uc->unitCore(C);
    d.check("xdev1: C, not open, has an extra core of the set: no tab, its serial held",
            !uc->unitInTab(C) && uc->unitCore(C) && !coreOfATab && !r->holder(C) && r->extraSerials() == QStringList{ C }
                && r->devices().size() == 3);
    d.spin(300);
    QQuickItem *noteB = in(QStringLiteral("MeasureUnits.unitTab.") + B);
    QQuickItem *noteC = in(QStringLiteral("MeasureUnits.unitTab.") + C);
    d.check("xdev1: the form says B is measured through its tab, and not C",
            noteB && noteB->isVisible() && noteB->property("text").toString().contains(sb->title()) && noteC && !noteC->isVisible());
    d.grab(w, QStringLiteral("xdev1/units-tab-and-extra.png"));

    // C cannot be opened in a tab while it is measured
    failed.clear();
    sys->openInTab(C);
    const bool refused = d.waitFor(20000, [&] { return failed.size() == 1 && r->opening() == 0; });
    const QString why = refused ? failed.at(0).at(1).toString() : QString();
    std::printf("smoke: xdev1: C in a tab: %s\n", qPrintable(why));
    d.check("xdev1: C in a tab is refused while it is measured, in words", refused && why.contains(QLatin1String("being measured"))
                                                                                && r->sessions().size() == 3 && !r->holder(C));

    // The check and a run: every unit's check on its own core
    rec.clear();
    steps.clear();
    uc->check();
    const bool checked = step(QStringLiteral("check"), 25000) && results.value(QStringLiteral("check"));
    d.check("xdev1: Check: B on its tab's core, C on its extra core, A on A's",
            checked && allAre(rec.of(B), sb->handle()) && allAre(rec.of(C), uc->unitCore(C)) && allAre(rec.of(A), sa->handle()));
    if (uc->simAvailable()) {
        rec.clear();
        steps.clear();
        uc->setSweepMs(1000);
        uc->start();
        const bool started = step(QStringLiteral("start"), 30000) && results.value(QStringLiteral("start"));
        std::printf("smoke: xdev1: start: %s %s\n", qPrintable(uc->error()), qPrintable(uc->phaseText()));
        d.check("xdev1: a run starts with the simulated microphone, B judged on its tab's core again",
                started && uc->running() && !mc->micOpen() && allAre(rec.of(B), sb->handle()) && allAre(rec.of(C), uc->unitCore(C)));
        uc->stop();
        d.check("xdev1: Stop ends it", d.waitFor(15000, [&] { return !uc->running() && !mc->running(); }));
    } else {
        d.check("xdev1: the demo hub offers its simulated microphone", false);
    }
    d.check("xdev1: B's tab is untouched by the run: still open and connected", sb && sb->model()->connected() && r->sessions().size() == 3);

    // Leaving the tab closes C's extra core (and not B's tab); C opens in a tab then
    leaveUnits();
    d.check("xdev1: leaving Several units closes C's extra core and gives its serial back",
            d.waitFor(10000, [&] { return r->extraSerials().isEmpty() && !uc->unitOpen(C); }));
    d.check("xdev1: B's tab stays", sb && sb->model()->connected() && r->sessions().size() == 3);
    failed.clear();
    d.check("xdev1: now C opens in a tab", sys->openInTab(C) && d.waitFor(20000, [&] {
        return r->opening() == 0 && r->holder(C) && r->holder(C)->model()->connected();
    }) && failed.isEmpty());
    QPointer<DeviceSession> sc = r->holder(C);
    r->activate(sa);
    d.waitIdle();

    // A tab closed while its unit is chosen: the unit has no connection now
    steps.clear();
    tabs->setProperty("currentIndex", 6);
    step(QStringLiteral("enter"), 10000);
    uc->setPlacement(QStringLiteral("listeningPosition"));
    uc->setUnitChosen(A, true);
    uc->setUnitChosen(C, true);
    d.check("xdev1: C now is judged by its new tab's core",
            d.waitFor(20000, [&] { return uc->unitOpen(C); }) && sc && uc->unitInTab(C) && uc->unitCore(C) == sc->handle()
                && r->extraSerials().isEmpty());
    if (sc) r->close(sc->id());
    d.waitFor(10000, [&] { return sc.isNull(); });
    d.spin(200);
    QVariantMap rowC;
    for (const QVariant &v : uc->units())
        if (v.toMap().value(QStringLiteral("serial")).toString() == C) rowC = v.toMap();
    d.check("xdev1: its tab closed: C has no connection, the row says to choose it again",
            !uc->unitOpen(C) && rowC.value(QStringLiteral("error")).toString().contains(QLatin1String("tab was closed"))
                && !uc->blocker().isEmpty());
    leaveUnits();
    link->closeSession();
}

void xdev1Section(SmokeDriver &d) {
    d.allowSectionTime(300);
    DeviceRegistry *r = DeviceRegistry::instance();
    auto *notices = Registrar::instance<NoticeCenter>();
    d.check("xdev1: the registry and the notices", r && notices);
    if (!r || !notices) return;
    demosPart(d, r, notices);
    hubPart(d, r, notices);
    d.check("xdev1: afterwards only the main session, no extra core", r->sessions().size() == 1 && r->extraSerials().isEmpty()
                                                                      && !r->keepOpen());
}

const SmokeDriver::Section section("xdev1-devices", 975, &xdev1Section);

} // namespace
