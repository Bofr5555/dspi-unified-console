// WP QD0: an import applies once however its reply fits, and the registrar and
// banner host work from a controller's own .cpp, as a later WP uses them.
#include "SmokeDriver.h"

#include "ConfigFiles.h"
#include "DSPiBridge.h"
#include "MonitorModel.h"
#include "Registrar.h"

#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickWindow>
#include <QUrl>
#include <cstdio>

class SeamProbe : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool bannerShown READ bannerShown WRITE setBannerShown NOTIFY bannerShownChanged)
public:
    SeamProbe(DSPiBridge *bridge, QObject *parent) : QObject(parent), m_bridge(bridge) {
        ++s_made;
    }
    DSPiBridge *bridge() const { return m_bridge; }
    bool bannerShown() const { return m_shown; }
    void setBannerShown(bool shown) {
        if (shown == m_shown) return;
        m_shown = shown;
        emit bannerShownChanged();
    }
    static int s_made;
signals:
    void bannerShownChanged();

private:
    DSPiBridge *m_bridge;
    bool m_shown = false;
};
int SeamProbe::s_made = 0;

static const Registrar::Controller<SeamProbe> probeRegistered("seamProbe");
static const Registrar::Banner probeBanner("qrc:/smoke/SeamProbeBanner.qml");
static const Registrar::Banner probeTopBanner("qrc:/smoke/SeamProbeTopBanner.qml", -1);

void SmokeDriver::testImportAppliesOnce() {
    section("QD0: one import applies once");
    auto *cf = m_.config;
    auto *b = m_.bridge;
    auto *mon = m_.monitor;
    mon->setWatching(true);
    // Each bulk write the device takes is announced once: the count of imports applied.
    const auto bulkWrites = [mon]() { return mon->allText().count(QStringLiteral("BulkInvalidated")); };
    const QUrl config = QUrl::fromLocalFile(m_out + QStringLiteral("/qd0.dspiconfig"));
    b->setMasterVolume(-26.0f);
    check("QD0: export", cf->exportConfiguration(config).value("ok").toBool());

    const auto runImport = [&]() {
        check("QD0: inspect", cf->inspectConfiguration(config).value("ok").toBool());
        b->setMasterVolume(-40.0f);
        spin(200);                  // that setter's own notification, before the clear
        mon->clear();
        const QVariantMap r = cf->importConfiguration(config, true, false);
        waitFor(1500, [&]() { return bulkWrites() > 0; });
        spin(400);                  // a second one would have arrived by now
        return r;
    };

    QVariantMap r = runImport();
    check("QD0: a reply that fits is ok", r.value("ok").toBool());
    printf("smoke: QD0 bulk writes, reply fits: %d\n", bulkWrites());
    check("QD0: a reply that fits: applied once", bulkWrites() == 1);
    check("QD0: a reply that fits: volume restored", qAbs(b->masterVolumeDB() + 26.0f) < 0.01f);

    cf->setApplyReplyCapacity(8);
    r = runImport();
    printf("smoke: QD0 bulk writes, reply too long: %d (%s)\n", bulkWrites(), qPrintable(r.value("error").toString()));
    check("QD0: a reply too long is a failure", !r.value("ok").toBool() && !r.value("error").toString().isEmpty());
    check("QD0: a reply too long: applied once", bulkWrites() == 1);
    check("QD0: a reply too long: state re-read", qAbs(b->masterVolumeDB() + 26.0f) < 0.01f);
    cf->setApplyReplyCapacity(64 * 1024);

    // A filter import goes through the same call. A second run would change nothing
    // the monitor shows, so this checks the outcome only.
    const QUrl filters = QUrl::fromLocalFile(m_out + QStringLiteral("/qd0.txt"));
    b->setFilter(0, 0, 1, 777.0f, 1.0f, 0.9f);
    check("QD0: filter export", cf->exportFilters(filters).value("ok").toBool());
    r = cf->inspectFilters(filters);
    QVariantList wires;
    for (const QVariant &c : r.value("channels").toList()) wires.append(c.toMap().value("wire"));
    b->setFilter(0, 0, 1, 333.0f, 1.0f, 0.9f);
    cf->setApplyReplyCapacity(8);
    r = cf->importFilters(wires);
    cf->setApplyReplyCapacity(64 * 1024);
    check("QD0: filter reply too long is a failure", !r.value("ok").toBool() && !r.value("error").toString().isEmpty());
    check("QD0: filter reply too long: state re-read", qAbs(b->filterFreq(0, 0) - 777.0f) < 0.5f);
    mon->clear();
    mon->setWatching(false);
}

void SmokeDriver::testSeam() {
    section("QD0: registrar and banner host");
    QQmlContext *context = m_.engine->rootContext();
    auto *probe = qobject_cast<SeamProbe *>(context->contextProperty(QStringLiteral("seamProbe")).value<QObject *>());
    check("QD0: controller made once, under its name", probe && SeamProbe::s_made == 1);
    check("QD0: controller got the bridge", probe && probe->bridge() == m_.bridge);
    const auto qmlType = [&](const char *name) {
        QQmlExpression e(context, root(), QStringLiteral("typeof %1").arg(QString::fromLatin1(name)));
        const QString type = e.evaluate().toString();
        return e.hasError() ? QStringLiteral("error") : type;
    };
    check("QD0: QML sees the controller", qmlType("seamProbe") == QLatin1String("object"));
    check("QD0: QML can test for one not there", qmlType("noSuchController") == QLatin1String("undefined"));
    check("QD0: banners sorted by order",
          // -- WP QD8 -- Other WPs' banners register too: these two's order is the test
          context->contextProperty(QStringLiteral("registeredBanners")).toStringList().filter(QStringLiteral("/smoke/SeamProbe"))
              == QStringList({ QStringLiteral("qrc:/smoke/SeamProbeTopBanner.qml"), QStringLiteral("qrc:/smoke/SeamProbeBanner.qml") }));

    auto *host = root()->findChild<QQuickItem *>(QStringLiteral("bannerHost"));
    auto *content = root()->findChild<QQuickItem *>(QStringLiteral("contentLoader"));
    check("QD0: banner host in the main window", host && content);
    if (!probe || !host || !content) return;
    const qreal before = content->height();
    // -- WP QD7 -- The demo's label is up on this run. The host with none shown is
    // checked by QD7's own run, without --demo.
    const qreal hostBefore = host->height();
    const int shownBefore = host->property("shownCount").toInt();

    probe->setBannerShown(true);
    check("QD0: banner shows", waitFor(1000, [&]() { return host->isVisible() && host->height() == hostBefore + 44.0 + 10.0; }));
    spin(100);
    check("QD0: the hidden banner is not counted", host->property("shownCount").toInt() == shownBefore + 1);
    check("QD0: the content makes room", content->height() == before - 44.0 - 10.0);
    if (auto *main = qobject_cast<QQuickWindow *>(root())) main->grabWindow().save(m_out + QStringLiteral("/window-banner.png"));

    probe->setBannerShown(false);
    check("QD0: banner hides", waitFor(1000, [&]() { return host->height() == hostBefore && host->property("shownCount").toInt() == shownBefore; }));
    spin(100);
    check("QD0: the content takes the room back", content->height() == before);
}

#include "SmokeDriverSeam.moc"
