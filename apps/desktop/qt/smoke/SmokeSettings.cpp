// -- WP QP10 -- The Settings window: it opens from its menu row and from Help >
// About, lists the pages in their groups, searches them, goes back and
// forward, remembers the last page, and its pages show and change what they
// should (About with the notices viewer, Advanced, Graphing, Spectrum
// Analyser, Updates with and without a checker, Overview's pins and core
// voltage). The registration API is exercised with a probe page, section and
// footer, as a later package would add them. Screenshots of each page.
#include "SmokeDriver.h"

#include "AboutController.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "GraphController.h"
#include "PrefsMap.h"
#include "SettingsController.h"
#include "Shell.h"

#include "CoreApi.h"

#include <QDateTime>
#include <QFile>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTest>

namespace {

// The probe page, section and footer (SmokeSettings*.qml)
const SettingsRegistry::Page probePage{ "smokeprobe", SettingsRegistry::System, 99, "Smoke Probe", "info",
                                        "SmokeSettingsPage", { "zebra crossing" } };
const SettingsRegistry::Section probeSection{ "advanced", 90, "SmokeSettingsSection" };
const SettingsRegistry::Footer probeFooter{ 99, "SmokeSettingsFooter" };

// A stand-in for QP17's update checker, with the properties UpdatesPage reads
class FakeUpdater : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool feedAvailable MEMBER feedAvailable CONSTANT)
    Q_PROPERTY(bool automatic MEMBER automatic NOTIFY changed)
    Q_PROPERTY(bool checking MEMBER checking NOTIFY changed)
    Q_PROPERTY(QDateTime lastChecked MEMBER lastChecked NOTIFY changed)
    Q_PROPERTY(bool available MEMBER available NOTIFY changed)
    Q_PROPERTY(QString releaseName MEMBER releaseName CONSTANT)
    Q_PROPERTY(QString releaseVersion MEMBER releaseVersion CONSTANT)
    Q_PROPERTY(QString releaseNotes MEMBER releaseNotes CONSTANT)
    Q_PROPERTY(bool firmwareIncluded MEMBER firmwareIncluded CONSTANT)
    Q_PROPERTY(QString feedHost MEMBER feedHost CONSTANT)
public:
    bool feedAvailable = true;
    bool automatic = false;
    bool checking = false;
    QDateTime lastChecked = QDateTime::currentDateTime();
    bool available = true;
    QString releaseName = QStringLiteral("Smoke release");
    QString releaseVersion = QStringLiteral("9.9.9");
    QString releaseNotes = QStringLiteral("<b>not bold</b>\nsecond line");
    bool firmwareIncluded = true;
    QString feedHost = QStringLiteral("example.invalid");
    int checks = 0, skips = 0, opens = 0;
    Q_INVOKABLE void checkNow() { ++checks; }
    Q_INVOKABLE void skipVersion() { ++skips; }
    Q_INVOKABLE void openRelease() { ++opens; }
signals:
    void changed();
};

QObject *find(QObject *root, const QString &name) {
    if (!root) return nullptr;
    if (root->objectName() == name) return root;
    if (auto *item = qobject_cast<QQuickItem *>(root)) {
        for (QQuickItem *child : item->childItems())
            if (QObject *o = find(child, name)) return o;
    }
    for (QObject *child : root->children())
        if (QObject *o = find(child, name)) return o;
    return nullptr;
}

struct Ui {
    SmokeDriver &d;
    QQuickWindow *w;
    QObject *get(const QString &name) const { return w ? find(w->contentItem(), name) : nullptr; }
    QObject *get(const char *name) const { return get(QString::fromLatin1(name)); }
    QString text(const char *name) const {
        QObject *o = get(name);
        return o ? o->property("text").toString() : QString();
    }
    bool shown(const char *name) const {
        auto *item = qobject_cast<QQuickItem *>(get(name));
        return item && item->isVisible();
    }
    void click(const char *name) const {
        if (QObject *o = get(name)) QMetaObject::invokeMethod(o, "click");
        d.spin(50);
    }
    // The sidebar's page ids, in order
    QStringList pages(SettingsController *s, const QString &query = {}) const {
        QStringList ids;
        for (const QVariant &row : s->sidebarRows(query)) {
            const QVariantMap m = row.toMap();
            if (m.contains(QStringLiteral("id"))) ids << m.value(QStringLiteral("id")).toString();
        }
        return ids;
    }
};

void showOnly(SmokeDriver &d, SettingsController *s, const char *page) {
    s->open(QString::fromLatin1(page));
    d.spin(300);
    d.waitIdle();
}

void settingsSection(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    DeviceModel *device = d.app().device;
    auto *s = Registrar::instance<SettingsController>();
    auto *about = Registrar::instance<AboutController>();
    d.check(QStringLiteral("qp10: the controllers exist"), s && about);
    if (!s || !about) return;
    s->setPageAvailable(QStringLiteral("smokeprobe"), false);

    // ---- The window and its page list -----------------------------------------
    shell->trigger(QStringLiteral("edit.settings"));
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check(QStringLiteral("qp10: Edit > Settings opens the window"),
            w && d.waitFor(3000, [&]() { return w->isVisible(); }) && w->objectName() == QLatin1String("SettingsWindow"));
    if (!w) return;
    Ui ui{ d, w };
    d.spin(300);
    d.check(QStringLiteral("qp10: a fresh profile opens on About"), s->currentPage() == QLatin1String("about"));
    d.check(QStringLiteral("qp10: the title row names the page"), ui.text("Settings.title") == s->pageTitle(QStringLiteral("about")));
    // -- WP QTPOP -- taller when the page needs it (About does)
    d.check(QStringLiteral("qp10: the default size is 860 x 630 or taller"), w->width() == 860 && w->height() >= 630);

    const QStringList all = ui.pages(s);
    d.check(QStringLiteral("qp10: the list holds the QP10 pages in the Mac's order"),
            all.indexOf(QStringLiteral("about")) == 0 && all.indexOf(QStringLiteral("updates")) == 1 &&
                all.indexOf(QStringLiteral("advanced")) == 2 &&
                all.indexOf(QStringLiteral("graphing")) > all.indexOf(QStringLiteral("advanced")) &&
                all.indexOf(QStringLiteral("spectrum")) == all.indexOf(QStringLiteral("graphing")) + 1 &&
                all.indexOf(QStringLiteral("overview")) > all.indexOf(QStringLiteral("spectrum")));
    int headings = 0;
    for (const QVariant &row : s->sidebarRows(QString())) headings += row.toMap().contains(QStringLiteral("heading"));
    d.check(QStringLiteral("qp10: Application, Display and System headings show with a device"), headings >= 3);
    d.check(QStringLiteral("qp10: a page row is in the sidebar"), ui.get("Settings.row.graphing") != nullptr);

    // ---- The registration API ----------------------------------------------------
    d.check(QStringLiteral("qp10: an unavailable page is not listed"), !all.contains(QStringLiteral("smokeprobe")));
    s->setPageAvailable(QStringLiteral("smokeprobe"), true);
    d.check(QStringLiteral("qp10: a registered page lists in its group, by order"),
            ui.pages(s).last() == QLatin1String("smokeprobe") ||
                ui.pages(s).indexOf(QStringLiteral("smokeprobe")) > ui.pages(s).indexOf(QStringLiteral("overview")));
    d.check(QStringLiteral("qp10: a registered page's terms are searched"),
            ui.pages(s, QStringLiteral("zebra")) == QStringList{ QStringLiteral("smokeprobe") });
    s->addSearchTerms(QStringLiteral("smokeprobe"), { QStringLiteral("okapi") });
    d.check(QStringLiteral("qp10: terms added at run time are searched"),
            ui.pages(s, QStringLiteral("OKAPI")) == QStringList{ QStringLiteral("smokeprobe") });
    showOnly(d, s, "smokeprobe");
    d.check(QStringLiteral("qp10: a registered page opens"), ui.get("SmokeSettingsPage") != nullptr);
    s->setPageAvailable(QStringLiteral("smokeprobe"), false);
    d.spin(200);
    d.check(QStringLiteral("qp10: the page showing falls back to About when it goes"), s->currentPage() == QLatin1String("about"));
    d.check(QStringLiteral("qp10: a registered footer shows on every page"), ui.get("SmokeSettingsFooter") != nullptr);

    // ---- Search (behaviour.md §9) ---------------------------------------------------
    QObject *search = ui.get("Settings.search");
    d.check(QStringLiteral("qp10: the search field exists"), search != nullptr);
    showOnly(d, s, "advanced");
    if (search) {
        search->setProperty("text", QStringLiteral("phase"));
        d.spin(150);
        d.check(QStringLiteral("qp10: \"phase\" leaves Graphing (a row, not a title)"),
                ui.pages(s, QStringLiteral("phase")) == QStringList{ QStringLiteral("graphing") });
        d.check(QStringLiteral("qp10: the page showing does not change while typing"), s->currentPage() == QLatin1String("advanced"));
        d.check(QStringLiteral("qp10: the clear button shows while the field holds text"), ui.shown("Settings.searchClear"));
        d.check(QStringLiteral("qp10: \"pin\" leaves Overview"), ui.pages(s, QStringLiteral("pin")).contains(QStringLiteral("overview")));
        d.check(QStringLiteral("qp10: matching ignores case"), ui.pages(s, QStringLiteral("PEAK HOLD")) == QStringList{ QStringLiteral("spectrum") });
        search->setProperty("text", QStringLiteral("qqqzzz"));
        d.spin(150);
        d.check(QStringLiteral("qp10: no match shows the one-line message"), ui.shown("Settings.noMatch") && ui.pages(s, QStringLiteral("qqqzzz")).isEmpty());
        search->setProperty("text", QStringLiteral("glow"));
        d.spin(150);
        d.grab(w, QStringLiteral("settings-search.png"));
        if (auto *field = qobject_cast<QQuickItem *>(search)) {
            w->requestActivate();
            field->forceActiveFocus();
            d.spin(50);
            QTest::keyClick(w, Qt::Key_Return);
            d.spin(150);
        }
        d.check(QStringLiteral("qp10: Return opens the first page left"), s->currentPage() == QLatin1String("graphing"));
        ui.click("Settings.searchClear");
        d.check(QStringLiteral("qp10: clearing restores the full list"),
                search->property("text").toString().isEmpty() && !ui.shown("Settings.noMatch"));
    }

    // ---- History (the Mac's back and forward) -----------------------------------------
    showOnly(d, s, "about");
    showOnly(d, s, "graphing");
    showOnly(d, s, "spectrum");
    ui.click("Settings.back");
    d.check(QStringLiteral("qp10: Back returns to the page before"), s->currentPage() == QLatin1String("graphing") && s->canGoForward());
    ui.click("Settings.forward");
    d.check(QStringLiteral("qp10: Forward goes on again"), s->currentPage() == QLatin1String("spectrum") && !s->canGoForward());
    ui.click("Settings.back");
    showOnly(d, s, "updates");
    d.check(QStringLiteral("qp10: a fresh navigation drops the forward pages"), !s->canGoForward() && s->canGoBack());
    d.check(QStringLiteral("qp10: the last page is remembered"),
            QSettings().value(QStringLiteral("settingsWindow/lastPage")).toString() == QLatin1String("updates"));

    // ---- About -----------------------------------------------------------------------
    showOnly(d, s, "about");
    d.check(QStringLiteral("qp10: About shows the version and the platform"),
            ui.text("About.version").contains(QStringLiteral(DSPI_APP_VERSION)) &&
                ui.text("About.version").contains(about->platformLabel()));
    d.check(QStringLiteral("qp10: About shows the core's credit lines"),
            !about->creditLines().isEmpty() && ui.text("About.credit.0") == about->creditLines().first());
    d.check(QStringLiteral("qp10: About lists the core's links"), !about->links().isEmpty() && ui.get("About.link.github") != nullptr);
    d.check(QStringLiteral("qp10: About lists the credits"), !about->credits().isEmpty());
    d.check(QStringLiteral("qp10: the notices are found beside the build"), about->notices().size() >= 5);
    d.grab(w, QStringLiteral("settings-about.png"));
    QObject *viewer = ui.get("About.viewer");
    d.check(QStringLiteral("qp10: the notice viewer exists"), viewer != nullptr);
    int gpl = -1;
    const QVariantList notices = about->notices();
    for (int i = 0; i < notices.size(); ++i)
        if (notices.at(i).toMap().value(QStringLiteral("path")).toString().endsWith(QLatin1String("GPL-3.0.txt"))) gpl = i;
    d.check(QStringLiteral("qp10: the GPL is one of the notices"), gpl >= 0);
    if (viewer && gpl >= 0) {
        ui.click(qPrintable(QStringLiteral("About.notice.%1").arg(gpl)));
        d.check(QStringLiteral("qp10: a notice row opens the viewer"),
                d.waitFor(3000, [&]() { return viewer->property("opened").toBool() && !about->noticeLoading(); }));
        QFile file(notices.at(gpl).toMap().value(QStringLiteral("path")).toString());
        const bool read = file.open(QIODevice::ReadOnly);
        d.check(QStringLiteral("qp10: the viewer shows the file's text as it is"),
                read && ui.text("About.viewerText") == QString::fromUtf8(file.readAll()));
        d.check(QStringLiteral("qp10: the viewer's title is the notice's name"),
                ui.text("About.viewerTitle") == notices.at(gpl).toMap().value(QStringLiteral("title")).toString());
        d.grab(w, QStringLiteral("about-notice.png"));
        ui.click("About.viewerClose");
        d.check(QStringLiteral("qp10: Close closes the viewer"), d.waitFor(2000, [&]() { return !viewer->property("visible").toBool(); }));
        ui.click(qPrintable(QStringLiteral("About.notice.%1").arg(gpl)));
        d.waitFor(2000, [&]() { return viewer->property("opened").toBool(); });
        QTest::keyClick(w, Qt::Key_Escape);
        d.check(QStringLiteral("qp10: Escape closes the viewer"), d.waitFor(2000, [&]() { return !viewer->property("visible").toBool(); }));
    }
    const Qt::ColorScheme schemeBefore = shell->colorSchemeOverride();
    shell->setColorSchemeOverride(Qt::ColorScheme::Light);
    d.spin(200);
    d.grab(w, QStringLiteral("settings-about-light.png"));
    shell->setColorSchemeOverride(schemeBefore);   // -- WP QPI3 --

    // Help > About opens the window on About
    showOnly(d, s, "graphing");
    shell->trigger(QStringLiteral("help.about"));
    d.spin(200);
    d.check(QStringLiteral("qp10: Help > About opens Settings on About"), s->currentPage() == QLatin1String("about") && w->isVisible());

    // ---- Graphing: a preference applies and is stored at once -------------------------
    showOnly(d, s, "graphing");
    // -- WP QPI1 -- the page's preferences are GraphController's (the one owner)
    GraphController *graph = Registrar::instance<GraphController>();
    const bool glow = graph->showGraphGlow();
    if (QObject *sw = ui.get("Graphing.showGlow.switch")) QMetaObject::invokeMethod(sw, "click");
    d.spin(100);
    d.check(QStringLiteral("qpi1: Graph Line Glow switches the graph's glow"), graph->showGraphGlow() == !glow);
    d.check(QStringLiteral("qp10: and stores it under graph/showGlow"),
            QSettings().value(QStringLiteral("graph/showGlow")).toBool() == !glow);
    graph->setShowGraphGlow(glow);
    d.spin(50);
    auto *glowRow = qobject_cast<QQuickItem *>(ui.get("Graphing.showGlow"));
    d.check(QStringLiteral("qpi1: the page follows a change made on the graph"),
            glowRow && glowRow->property("checked").toBool() == glow);
    auto *unwrap = qobject_cast<QQuickItem *>(ui.get("Graphing.phaseUnwrapped"));
    d.check(QStringLiteral("qp10: Unwrap Phase is off while phase is hidden"),
            unwrap && (graph->showPhase() || !unwrap->isEnabled()));
    const double minFreq = graph->minFreq(), maxFreq = graph->maxFreq(), range = graph->dbRange(), centre = graph->dbCenter();
    graph->resetScale();
    auto *lineWidth = qobject_cast<QQuickItem *>(ui.get("Graphing.lineWidth"));
    d.check(QStringLiteral("qpi1: the graph defaults are behaviour.md's"),
            graph->dbRange() == 50 && graph->minFreq() == 15 && graph->maxFreq() == 20000 && graph->dbCenter() == 0 &&
                lineWidth && lineWidth->property("resetValue").toDouble() == 1.5);
    graph->setMinFreq(minFreq);
    graph->setMaxFreq(maxFreq);
    graph->setDbRange(range);
    graph->setDbCenter(centre);
    d.grab(w, QStringLiteral("settings-graphing.png"));
    if (QObject *sw = ui.get("Graphing.showGlow.switch")) QMetaObject::invokeMethod(sw, "click");
    d.spin(50);

    // ---- Spectrum Analyser -------------------------------------------------------------
    showOnly(d, s, "spectrum");
    d.check(QStringLiteral("qp10: Spectrum reads the analyser's range"),
            d.waitFor(3000, [&]() { return s->analyser().contains(QStringLiteral("supported")); }));
    QObject *order = ui.get("Spectrum.fftOrder.combo");
    d.check(QStringLiteral("qp10: Transform Size offers the device's sizes"),
            order && order->property("count").toInt() >= 1 && order->property("currentIndex").toInt() >= 0);
    d.check(QStringLiteral("qp10: the spectrum defaults are behaviour.md's"),
            s->spectrum()->fallback(QStringLiteral("fftOrder")).toInt() == 10 &&
                s->spectrum()->fallback(QStringLiteral("floorDb")).toDouble() == -90);
    d.grab(w, QStringLiteral("settings-spectrum.png"));

    // ---- Updates: without a checker, then with one ---------------------------------------
    showOnly(d, s, "updates");
    auto enabled = [&](const char *name) {
        auto *item = qobject_cast<QQuickItem *>(ui.get(name));
        return item && item->isEnabled();
    };
    d.check(QStringLiteral("qp10: without a feed, Check Now and the switch are off"),
            !enabled("Updates.checkNow") && !enabled("Updates.automatic.switch"));
    d.check(QStringLiteral("qp10: and the page says it never checks"), ui.text("Updates.detail").contains(QStringLiteral("never")));
    d.check(QStringLiteral("qp10: the privacy line always shows"), ui.shown("Updates.privacy") && !ui.text("Updates.privacy").isEmpty());
    d.grab(w, QStringLiteral("settings-updates.png"));
    FakeUpdater updater;
    s->setUpdater(&updater);
    d.spin(200);
    d.check(QStringLiteral("qp10: with a feed, Check Now and the switch are on"), enabled("Updates.checkNow") && enabled("Updates.automatic.switch"));
    ui.click("Updates.checkNow");
    d.check(QStringLiteral("qp10: Check Now asks the checker"), updater.checks == 1);
    if (QObject *sw = ui.get("Updates.automatic.switch")) QMetaObject::invokeMethod(sw, "click");
    d.spin(50);
    d.check(QStringLiteral("qp10: the switch sets automatic checks"), updater.automatic);
    d.check(QStringLiteral("qp10: a newer release shows the Available card"), ui.shown("Updates.availableCard"));
    d.check(QStringLiteral("qp10: release notes show as plain text"), ui.text("Updates.notes") == updater.releaseNotes);
    d.check(QStringLiteral("qp10: the privacy line names the feed's host"), ui.text("Updates.privacy").contains(updater.feedHost));
    ui.click("Updates.viewRelease");
    d.check(QStringLiteral("qp10: View Release asks the checker"), updater.opens == 1);
    d.grab(w, QStringLiteral("settings-updates-available.png"));
    ui.click("Updates.skip");
    d.spin(200);
    // The question's accept button
    QObject *accept = find(ui.get("Updates.skipQuestion"), QStringLiteral("QuestionDialog.accept"));
    if (accept) QMetaObject::invokeMethod(accept, "click");
    d.spin(200);
    d.check(QStringLiteral("qp10: Skip This Version asks, then skips"), updater.skips == 1);
    s->setUpdater(nullptr);
    d.spin(100);

    // ---- Advanced ---------------------------------------------------------------------
    showOnly(d, s, "advanced");
    d.check(QStringLiteral("qp10: Advanced shows the firmware"), ui.text("Advanced.firmware.value") == device->firmwareVersion());
    d.check(QStringLiteral("qp10: a registered section shows on its page"), ui.get("SmokeSettingsSection") != nullptr);
    const bool debug = s->showDebugInfo();
    if (QObject *sw = ui.get("Advanced.debug.switch")) QMetaObject::invokeMethod(sw, "click");
    d.spin(50);
    d.check(QStringLiteral("qp10: Show Debug Information switches and is stored"),
            s->showDebugInfo() == !debug && QSettings().value(QStringLiteral("advanced/showDebugInfo")).toBool() == !debug);
    s->setShowDebugInfo(debug);
    d.grab(w, QStringLiteral("settings-advanced.png"));
    // Reset Channel Names: every name back to the firmware's default
    const DspState *state = device->state();
    if (state) {
        const int channels = state->num_channels;
        QStringList before;
        for (int ch = 0; ch < channels; ++ch)
            before << QString::fromUtf8(reinterpret_cast<const char *>(state->channel_names[ch]));
        device->setChannelName(0, QStringLiteral("QP10 renamed"));
        d.waitIdle();
        ui.click("Advanced.resetNames");
        d.waitIdle();
        d.spin(200);
        d.waitIdle();
        state = device->state();
        const auto name = [&](int ch) { return QString::fromUtf8(reinterpret_cast<const char *>(state->channel_names[ch])); };
        d.check(QStringLiteral("qp10: Reset gives the first input its default name"),
                state && name(0) == (state->input_source == 0 ? QStringLiteral("USB 1") : name(0)) && name(0) != QLatin1String("QP10 renamed"));
        d.check(QStringLiteral("qp10: and the PDM sub its default name"), state && name(channels - 1) == QLatin1String("PDM"));
        for (int ch = 0; ch < channels; ++ch) device->setChannelName(ch, before.at(ch));
        d.waitIdle();
    }

    // ---- Overview -----------------------------------------------------------------------
    showOnly(d, s, "overview");
    d.check(QStringLiteral("qp10: the demo holds pins"), d.waitFor(3000, [&]() { return !s->pinClaims().isEmpty(); }));
    QObject *grid = ui.get("Overview.pinGrid");
    d.check(QStringLiteral("qp10: the map shows every valid GPIO"), grid && grid->property("pins").toList().size() == 26);
    d.check(QStringLiteral("qp10: the map counts the held pins"), grid && grid->property("heldCount").toInt() == s->pinClaims().size());
    d.check(QStringLiteral("qp10: the summary says how many are in use"),
            ui.text("Overview.inUse") == QStringLiteral("%1 of 26 GPIOs in use").arg(s->pinClaims().size()));
    bool outputs = false, clocks = false;
    for (const QVariant &c : s->pinClaims()) {
        outputs |= c.toMap().value(QStringLiteral("role")).toInt() == 0;
        clocks |= c.toMap().value(QStringLiteral("label")).toString() == QLatin1String("I2S BCK");
    }
    d.check(QStringLiteral("qp10: the outputs and the I2S clock are held"), outputs && clocks);
    d.check(QStringLiteral("qp10: the held pins group by role"), ui.shown("Overview.role.0") && ui.shown("Overview.legend"));
    d.check(QStringLiteral("qp10: the core voltage extension is asked about"),
            d.waitFor(3000, [&]() { return s->coreVoltage().contains(QStringLiteral("supported")); }));
    if (s->coreVoltage().value(QStringLiteral("supported")).toBool()) {
        d.check(QStringLiteral("qp10: the core voltage status is read"),
                d.waitFor(3000, [&]() { return s->coreVoltage().value(QStringLiteral("read")).toBool(); }));
        d.spin(100);
        d.check(QStringLiteral("qp10: the core voltage block shows the running voltage"),
                ui.shown("Overview.coreVoltage") && ui.text("Overview.running.value").endsWith(QLatin1String(" V")));
    }
    d.check(QStringLiteral("qp10: volts follow the Mac's table"),
            s->voltsLabel(0x0D) == QLatin1String("1.20 V") && s->voltsLabel(0x40).startsWith(QLatin1String("code 0x40")));
    d.check(QStringLiteral("qp10: uptimes read as the Mac's"),
            s->uptimeLabel(42) == QLatin1String("42 s") && s->uptimeLabel(3 * 3600 + 12 * 60) == QLatin1String("3 h 12 min"));
    d.grab(w, QStringLiteral("settings-overview.png"));

    // ---- Nothing connected: System pages go, the page showing falls back to About -----
    d.app().core->worker()->post([](FfiCore *c) { dspi_disconnect(c); });
    device->refresh();
    d.check(QStringLiteral("qp10: the device closes"), d.waitFor(3000, [&]() { return !device->connected(); }));
    d.spin(200);
    d.check(QStringLiteral("qp10: without a device the System pages are not listed"),
            !ui.pages(s).contains(QStringLiteral("overview")) && ui.pages(s).contains(QStringLiteral("graphing")));
    d.check(QStringLiteral("qp10: and Overview falls back to About"), s->currentPage() == QLatin1String("about"));
    device->openDemo();
    d.check(QStringLiteral("qp10: the demo opens again"), d.waitFor(5000, [&]() { return device->connected(); }));
    d.waitIdle();
    d.check(QStringLiteral("qp10: the System pages are back"), ui.pages(s).contains(QStringLiteral("overview")));

    w->close();
    d.spin(100);
}

const SmokeDriver::Section settings("qp10-settings", 40, &settingsSection);

} // namespace

#include "SmokeSettings.moc"
