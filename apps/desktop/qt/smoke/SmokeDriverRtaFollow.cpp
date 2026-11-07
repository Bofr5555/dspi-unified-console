// WP QD19: the analyser window over DSPi Link, when another console drives the
// device's analyser. The app's Link session and a second console (a core and a
// controller of its own, as SmokeDriverLink.cpp's Run) share one fake hub
// (crates/dspi-link's dspi-link-fakehub); the last to choose drives.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "LinkController.h"
#include "RtaController.h"
#include "TokenStore.h"

#include <QDir>
#include <QProcess>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>
#include <memory>

namespace {
// One dspi-link-fakehub process (SmokeDriverLink.cpp has the one with faults)
class FollowHub {
public:
    bool start() {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_FAKEHUB");
        if (binary.isEmpty()) return false;
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, { "--bind", "127.0.0.1:0", "--rta", "--auth", "none" });
        if (!m_proc.waitForStarted(5000) || !m_proc.waitForReadyRead(5000)) return false;
        const auto match = QRegularExpression(QStringLiteral(" on ([0-9.:]+) id ")).match(QString::fromUtf8(m_proc.readLine()));
        if (!match.hasMatch()) return false;
        url = QStringLiteral("ws://%1/dspi/v1").arg(match.captured(1));
        return true;
    }
    ~FollowHub() {
        m_proc.kill();
        m_proc.waitForFinished(2000);
    }
    QString url;

private:
    QProcess m_proc;
};

QQuickItem *findNamed(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = findNamed(c, name)) return found;
    return nullptr;
}

QList<int> ints(const QVariantList &l) {
    QList<int> out;
    for (const QVariant &v : l) out.append(v.toInt());
    return out;
}
}

void SmokeDriver::testRtaFollowing() {
    section("QD19: analyser window, following another console over Link");
    auto *app = qobject_cast<LinkController *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("linkSession")).value<QObject *>());
    auto *b = m_.bridge;
    auto *rta = m_.rta;
    FollowHub hub;
    const bool up = app && hub.start();
    check("QD19: the fake hub runs (DSPI_SMOKE_FAKEHUB)", up);
    if (!up) return;
    const auto settled = [this](LinkController &l) { return waitFor(30000, [&] { return !l.busy(); }); };
    const auto bind = [&](LinkController &l) {
        l.connectToHub(hub.url);
        if (!settled(l) || !waitFor(3000, [&] { return l.phase() == QLatin1String("choose_device") && l.devices().size() == 1; }))
            return false;
        const QVariantMap d = l.devices().value(0).toMap();
        l.bindDevice(d.value(QStringLiteral("handle")).toInt(), d.value(QStringLiteral("serial")).toString());
        return settled(l) && waitFor(5000, [&] { return l.phase() == QLatin1String("bound"); });
    };

    // The app on the hub's device, with the analyser window open
    const bool demoBefore = b->connected();
    check("QD19: the app binds the hub's device", bind(*app) && b->connected());
    check("QD19: its analyser probed", waitFor(5000, [&] { return rta->supported(); }));
    // main.qml names its windows by id only
    QQuickWindow *win = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->title() == QLatin1String("Spectrum Analyser")) win = w;
    check("QD19: the analyser window", win != nullptr);
    if (!win) return;
    QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("spectrum")));
    auto *followingText = findNamed(win->contentItem(), QStringLiteral("rtaFollowingText"));
    auto *takeControl = findNamed(win->contentItem(), QStringLiteral("rtaTakeControl"));
    check("QD19: the window has the following line and its button", followingText && takeControl);
    const QVariantMap own = win->property("ownSel").toMap();
    const int ownTap = own.value(QStringLiteral("tap")).toInt();
    printf("smoke: QD19 the window's own selection: tap %d, %d channels\n", ownTap, int(own.value(QStringLiteral("channels")).toList().size()));
    check("QD19: driving at first", waitFor(3000, [&] { return rta->running() && !rta->following(); }));
    check("QD19: no following line while driving", followingText && !followingText->isVisible());

    // A second console chooses other channels at the other tap: it drives now
    const QString scratch = qEnvironmentVariable("DSPI_SMOKE_OUT") + QStringLiteral("/rta-follow");
    QDir().mkpath(scratch);
    FfiCore *otherCore = dspi_core_new();
    auto other = std::make_unique<LinkController>(otherCore, std::make_unique<TokenStore>(nullptr, scratch + QStringLiteral("/tokens.json")), nullptr);
    check("QD19: the other console binds the same device", bind(*other));
    const int otherTap = ownTap == RTA_TAP_INPUT ? RTA_TAP_OUTPUT : RTA_TAP_INPUT;
    RtaRequest theirs = {};
    theirs.tap = uint8_t(otherTap);
    theirs.mask = 0b11;
    theirs.fft_order = 10;
    theirs.avg_ms = 300;
    theirs.peak_decay_db_s = 12;
    dspi_rta_set_request(otherCore, &theirs);
    // No bridge here to start it, as the app's does on a bind (DSPiBridge::adoptDevice)
    dspi_rta_start(otherCore);
    check("QD19: the other console's analyser probed",
          waitFor(5000, [&] { RtaCapsInfo c; dspi_rta_get_caps(otherCore, &c); return c.supported; }));
    const auto otherFollows = [&] {
        RtaOwnership o;
        dspi_rta_get_ownership(otherCore, &o);
        return o.following;
    };
    check("QD19: the app follows", waitFor(10000, [&] { return rta->following(); }));
    check("QD19: the device's tap and channels", rta->followedTap() == otherTap && ints(rta->followedChannels()) == QList<int>({ 0, 1 }));
    printf("smoke: QD19 following session %d\n", rta->followedOrigin());
    check("QD19: the window shows the device's selection", [&] {
        const QVariantMap sel = win->property("sel").toMap();
        return sel.value(QStringLiteral("tap")).toInt() == otherTap && ints(sel.value(QStringLiteral("channels")).toList()) == QList<int>({ 0, 1 });
    }());
    check("QD19: and says so", waitFor(2000, [&] { return followingText->isVisible() && takeControl->isVisible(); }));
    check("QD19: in the reference's words", followingText->property("text").toString() == QLatin1String("Following another console")
                                                && takeControl->property("text").toString() == QLatin1String("Take control"));
    check("QD19: the picture is the device's",
          waitFor(5000, [&] { return rta->snapshot().tap == otherTap && rta->snapshot().count == 2; }));
    // Showing the other console's channels is not a choice: nothing is pushed
    // over it, so it keeps driving
    spin(1500);
    check("QD19: the window takes nothing back", rta->following() && !otherFollows());
    win->grabWindow().save(m_out + QStringLiteral("/window-spectrum-following.png"));

    // Take control: the app pushes its own choice again, and the other follows
    QMetaObject::invokeMethod(takeControl, "clicked");
    check("QD19: Take control drives at once", !rta->following());
    check("QD19: the window shows its own selection again", win->property("sel").toMap().value(QStringLiteral("tap")).toInt() == ownTap);
    check("QD19: the line goes", !followingText->isVisible());
    check("QD19: the other console follows the app", waitFor(10000, otherFollows));
    check("QD19: the picture is the app's own", waitFor(5000, [&] { return rta->snapshot().tap == ownTap; }));
    spin(1500);
    check("QD19: no fight afterwards", !rta->following() && otherFollows());

    // A new choice there takes it back again, and the app follows once more
    theirs.mask = 0b1;
    dspi_rta_set_request(otherCore, &theirs);
    check("QD19: a new choice there: the app follows again",
          waitFor(10000, [&] { return rta->following() && ints(rta->followedChannels()) == QList<int>({ 0 }); }));

    other->closeSession();
    other.reset();
    dspi_core_free(otherCore);
    win->setVisible(false);
    app->closeSession();
    check("QD19: the app leaves the hub", waitFor(3000, [&] { return app->phase() == QLatin1String("idle"); }));
    check("QD19: back to what a scan finds", waitFor(3000, [&] { return b->connected() == demoBefore; }));
}
