// WP QD28: Settings > Advanced > Core Voltage on the demo device, with its simulated
// hangs, reboots and power cycles: every phase of the core's flow shows on the page,
// no write leaves without the confirmation card (and the word above 1.20 V), and the
// page is reachable from Advanced only.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "ExtController.h"
#include "VregController.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSet>
#include <QVariant>
#include <QVariantMap>
#include <cstdio>

namespace {

QQuickItem *findNamed(QQuickItem *in, const QString &name) {
    if (in->objectName() == name) return in;
    for (QQuickItem *child : in->childItems())
        if (QQuickItem *found = findNamed(child, name)) return found;
    return nullptr;
}

// The page's name for each of the core's phases
QString phaseTitle(const QString &key) {
    static const QVariantMap titles {
        { "idle", "Not started" }, { "review", "Ready" }, { "clearing", "Clearing the record" },
        { "starting_trial", "Starting the trial" }, { "soaking", "Soaking" }, { "passed", "Passed" },
        { "failed", "Failed" }, { "unknown", "Unknown" }, { "trial_found", "Trial found" },
        { "trial_left_running", "Trial left running" }, { "ending", "Ending the trial" }, { "ended", "Ended" },
        { "committing", "Saving" }, { "committed", "Saved" }, { "verify_failed", "Not confirmed" },
    };
    return titles.value(key).toString();
}

constexpr int kAllPhases = 15;
constexpr uint8_t k120 = 0x0D, k125 = 0x0E, k130 = 0x0F;
constexpr uint32_t kSoakS = 30 * 60;

} // namespace

void SmokeDriver::testVregCalibration() {
    section("QD28: core voltage calibration");
    auto *b = m_.bridge;
    QQmlContext *context = m_.engine->rootContext();
    auto *vreg = qobject_cast<VregController *>(context->contextProperty(QStringLiteral("vreg")).value<QObject *>());
    auto *ext = qobject_cast<ExtController *>(context->contextProperty(QStringLiteral("ext")).value<QObject *>());
    QQuickWindow *settings = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
    check("QD28: controller and window", vreg && ext && settings);
    if (!vreg || !ext || !settings) return;

    // A fresh demo device: nothing raised or on record from the passes before
    b->disconnectDevice();
    b->rescan();
    check("QD28: a fresh demo device", b->connected());

    const auto navigate = [&](const char *id) {
        QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QString::fromLatin1(id)));
        spin(150);
    };
    // Looked up each time: pages are built anew on each visit
    const auto item = [&](const char *name) { return findNamed(settings->contentItem(), QString::fromLatin1(name)); };
    const auto shows = [&](const char *name) { QQuickItem *i = item(name); return i && i->isVisible(); };
    const auto value = [&](const char *name) { QQuickItem *i = item(name); return i ? i->property("value").toString() : QStringLiteral("<no row>"); };
    const auto title = [&](const char *name) { QQuickItem *i = item(name); return i ? i->property("title").toString() : QStringLiteral("<no row>"); };
    // A click on a shown, enabled button row; false if the user couldn't make it
    const auto press = [&](const char *name) {
        QQuickItem *i = item(name);
        if (!i || !i->isVisible() || !i->isEnabled()) return false;
        QMetaObject::invokeMethod(i, "clicked");
        spin(30);
        return true;
    };
    // What the device says, read past the controller; uptime left out
    const auto device = [b]() {
        QByteArray reply(512, '\0');
        const uint32_t n = dspi_fetch_core_vreg(b->core(), reply.data(), uint32_t(reply.size()));
        reply.truncate(n > 0 ? int(n) - 1 : 0);
        QJsonObject o = QJsonDocument::fromJson(reply).object();
        o.remove(QStringLiteral("uptimeS"));
        return o;
    };
    const auto deviceInt = [&](const char *key) { return device().value(QLatin1String(key)).toInt(); };
    const auto idle = [&]() { return !vreg->busy(); };
    // A poll, as the timer would make it, and its view in
    const auto pollNow = [&]() {
        vreg->refresh();
        spin(60);
        waitFor(1000, idle);
        spin(30);
    };
    QSet<QString> seen;
    // The flow is in `key`, and the page says so
    const auto at = [&](const char *key) {
        const QString k = QString::fromLatin1(key);
        const bool ok = waitFor(1500, [&]() { return vreg->phase() == k && value("vregPhase") == phaseTitle(k); });
        if (ok) seen.insert(k);
        else printf("smoke: QD28 wanted %s, flow %s, page \"%s\", message \"%s\", error \"%s\"\n", key, qPrintable(vreg->phase()),
                    qPrintable(value("vregPhase")), qPrintable(title("vregMessage")), qPrintable(vreg->lastError()));
        return ok;
    };
    const auto grab = [&](const char *name) {
        settings->grabWindow().save(m_out + QStringLiteral("/settings-core-voltage-%1.png").arg(QString::fromLatin1(name)));
    };
    const auto confirmShows = [&]() { return shows("vregConfirm") && !vreg->pending().isEmpty(); };

    // ── Reachable from Advanced only ──
    settings->setVisible(true);
    spin(200);
    QVariant found;
    QMetaObject::invokeMethod(settings, "findPage", Q_RETURN_ARG(QVariant, found), Q_ARG(QVariant, QStringLiteral("coreVoltage")));
    check("QD28: no page of its own in the window", !found.isValid() || found.isNull() || !found.toMap().contains(QStringLiteral("source")));
    bool elsewhere = false;
    for (const char *id : { "about", "graphing", "spectrum", "overview", "inputs", "outputs", "i2s", "global", "surfaces", "control",
                            "groups", "macros", "aux" }) {
        navigate(id);
        if (item("coreVoltagePage") || item("advancedCoreVoltage") || vreg->watching()) {
            printf("smoke: QD28 core voltage controls on page %s\n", id);
            elsewhere = true;
        }
    }
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w != settings && findNamed(w->contentItem(), QStringLiteral("coreVoltagePage"))) elsewhere = true;
    check("QD28: on no other page or window", !elsewhere);
    check("QD28: polled once on connect, with no page open", waitFor(2000, [&]() { return vreg->polled(); }) && !vreg->watching());
    check("QD28: the demo device offers it", vreg->available() && vreg->phase() == QLatin1String("idle"));

    navigate("advanced");
    check("QD28: Advanced offers it", shows("advancedCoreVoltage") && shows("advancedCoreVoltageOpen") && !item("coreVoltagePage"));
    check("QD28: and is not reading", !vreg->watching());
    grab("advanced");

    // ── The page ──
    vreg->setPollPaused(true);  // one poll per step, from here
    check("QD28: Open", press("advancedCoreVoltageOpen"));
    check("QD28: the page opens in place", waitFor(1000, [&]() { return shows("coreVoltagePage") && !shows("advancedCoreVoltage"); }));
    check("QD28: watching while it shows", vreg->watching());
    pollNow();
    check("QD28: idle", at("idle"));
    check("QD28: the device's status", value("vregRunning") == QLatin1String("1.20 V") && value("vregStored") == QLatin1String("Build default")
                                           && value("vregHangs") == QLatin1String("0") && value("vregFailure") == QLatin1String("None"));
    check("QD28: the warning, with the datasheet limits", shows("vregWarning") && title("vregWarningText").contains(QLatin1String("1.21 V"))
                                                              && title("vregWarningText").contains(QLatin1String("1.16 V")));
    check("QD28: no confirmation card yet", !shows("vregConfirm"));
    grab("idle");

    // ── No write without the confirmation ──
    const QJsonObject before = device();
    const auto untouched = [&]() { return device() == before; };
    struct Staged { const char *button; const char *action; bool word; };
    for (const Staged &s : { Staged { "vregManualTrial", "trial", false }, Staged { "vregManualSet", "set", false },
                             Staged { "vregReset", "reset", false }, Staged { "vregAuto", "auto", false },
                             Staged { "vregClear", "clear", false } }) {
        const QString what = QStringLiteral("QD28: %1 is only staged").arg(QLatin1String(s.button));
        check(qPrintable(what), press(s.button) && confirmShows() && vreg->pending().value(QStringLiteral("action")).toString() == QLatin1String(s.action));
        spin(100);
        check(qPrintable(what + QStringLiteral(": nothing sent")), untouched());
        check(qPrintable(what + QStringLiteral(": Cancel drops it")), press("vregCancel") && !confirmShows() && untouched());
    }
    check("QD28: confirm with nothing staged sends nothing", !vreg->confirm(QStringLiteral("OVERVOLT")) && untouched());
    check("QD28: a voltage the flow doesn't offer is refused here", !vreg->request(QStringLiteral("trial"), k130) && !vreg->lastError().isEmpty()
                                                                        && !confirmShows() && untouched());
    check("QD28: below the build default too", !vreg->request(QStringLiteral("set"), 0x0C) && untouched());
    check("QD28: a start outside the flow is refused here", !vreg->request(QStringLiteral("start"), k120, kSoakS) && untouched());

    // 1.25 V needs the word: the card asks, Confirm waits for it, a wrong one is the core's to refuse
    QQuickItem *manualVoltage = item("vregManualVoltage");
    check("QD28: choices are the build default to 1.25 V", manualVoltage && manualVoltage->property("options").toList().size() == 2);
    QQuickItem *page = item("coreVoltagePage");
    if (page) page->setProperty("manualCode", k125);
    check("QD28: 1.25 V trial staged", press("vregManualTrial") && confirmShows() && vreg->pending().value(QStringLiteral("needsWord")).toBool());
    check("QD28: the word field shows", shows("vregWord"));
    check("QD28: Confirm waits for the word", item("vregConfirmButton") && !item("vregConfirmButton")->isEnabled());
    grab("confirm-word");
    if (QQuickItem *w = item("vregWord")) w->setProperty("text", QStringLiteral("overvolt"));
    check("QD28: a wrong word: sent to the core", press("vregConfirmButton"));
    check("QD28: and refused there, with nothing written", waitFor(1000, idle) && untouched() && !vreg->lastError().isEmpty());
    printf("smoke: QD28 wrong word: %s\n", qPrintable(vreg->lastError()));
    check("QD28: the error shows", shows("vregError"));
    check("QD28: staged again", press("vregManualTrial") && confirmShows());
    check("QD28: a staged request isn't sent by a poll", (pollNow(), untouched()));
    check("QD28: the word is typed afresh", item("vregWord") && item("vregWord")->property("text").toString().isEmpty());
    if (QQuickItem *w = item("vregWord")) w->setProperty("text", QStringLiteral("OVERVOLT"));
    check("QD28: with the word it goes", press("vregConfirmButton") && waitFor(1000, idle));
    check("QD28: a 1.25 V trial on the device", deviceInt("trialVreg") == k125 && device().value(QStringLiteral("trialActive")).toBool());
    pollNow();
    check("QD28: trial found", at("trial_found") && shows("vregKeep") && shows("vregEnd"));
    check("QD28: Advanced is told", vreg->view().value(QStringLiteral("status")).toMap().value(QStringLiteral("trialActive")).toBool());
    grab("trial-found");
    check("QD28: End Trial goes at once", press("vregEnd") && waitFor(1000, idle) && !confirmShows());
    check("QD28: ending", at("ending"));
    pollNow();
    check("QD28: ended", at("ended") && !device().value(QStringLiteral("trialActive")).toBool() && value("vregRunning") == QLatin1String("1.20 V"));

    // The manual requests reach the device once confirmed
    check("QD28: auto off staged", press("vregAuto") && confirmShows());
    check("QD28: auto off", press("vregConfirmButton") && waitFor(1000, idle) && device().value(QStringLiteral("autoDisabled")).toBool());
    pollNow();
    check("QD28: auto on", press("vregAuto") && press("vregConfirmButton") && waitFor(1000, idle) && !device().value(QStringLiteral("autoDisabled")).toBool());
    check("QD28: a hang", dspi_core_vreg_demo_event(b->core(), 0, 0) == 0);
    pollNow();
    check("QD28: shows on the page", value("vregHangs") == QLatin1String("1"));
    check("QD28: clear", press("vregClear") && press("vregConfirmButton") && waitFor(1000, idle) && deviceInt("hangCount") == 0);
    pollNow();
    check("QD28: cleared on the page", value("vregHangs") == QLatin1String("0"));
    if ((page = item("coreVoltagePage"))) page->setProperty("manualCode", k125);
    check("QD28: a 1.25 V save asks for the word", press("vregManualSet") && vreg->pending().value(QStringLiteral("needsWord")).toBool());
    if (QQuickItem *w = item("vregWord")) w->setProperty("text", QStringLiteral("OVERVOLT"));
    check("QD28: saved with it", press("vregConfirmButton") && waitFor(1000, idle) && at("committing"));
    pollNow();
    check("QD28: saved and read back", at("committed") && deviceInt("storedSel") == k125 && value("vregStored") == QLatin1String("1.25 V"));
    grab("committed");
    check("QD28: reset", press("vregReset") && confirmShows() && !vreg->pending().value(QStringLiteral("needsWord")).toBool()
                             && press("vregConfirmButton") && waitFor(1000, idle));
    pollNow();
    check("QD28: back to the build default", at("committed") && deviceInt("storedSel") == 0 && value("vregStored") == QLatin1String("Build default"));
    check("QD28: close", press("vregClose") && waitFor(1000, idle) && at("idle"));

    // ── The calibration ──
    check("QD28: a hang on record", dspi_core_vreg_demo_event(b->core(), 0, 0) == 0);
    check("QD28: Calibrate sends nothing", press("vregOpen") && waitFor(1000, idle) && deviceInt("hangCount") == 1);
    check("QD28: review", at("review") && title("vregMessage") == vreg->view().value(QStringLiteral("message")).toString());
    check("QD28: the record before it is cleared", value("vregHangs") == QLatin1String("1"));
    check("QD28: the soak suggested", item("vregSoakChoice") && item("vregSoakChoice")->property("value").toInt() == 30);
    grab("review");
    const QJsonObject reviewed = device();
    check("QD28: Start is only staged", press("vregStart") && confirmShows() && vreg->pending().value(QStringLiteral("value")).toInt() == k120
                                            && vreg->pending().value(QStringLiteral("soakS")).toInt() == int(kSoakS));
    spin(100);
    check("QD28: nothing sent before Confirm", device() == reviewed);
    check("QD28: 1.20 V needs no word", !vreg->pending().value(QStringLiteral("needsWord")).toBool() && !shows("vregWord"));
    check("QD28: confirmed", press("vregConfirmButton") && waitFor(1000, idle));
    check("QD28: clearing", at("clearing") && shows("vregEnd"));
    pollNow();
    check("QD28: starting the trial", at("starting_trial"));
    pollNow();
    check("QD28: soaking", at("soaking") && shows("vregSoak") && item("vregManual") && !item("vregManual")->isEnabled());
    check("QD28: the record was cleared first", deviceInt("hangCount") == 0 && deviceInt("trialVreg") == k120);
    check("QD28: soak time on the device", dspi_core_vreg_demo_event(b->core(), 3, 600) == 0);
    pollNow();
    check("QD28: the soak counts", value("vregSoak") == QLatin1String("10 min 0 s of 30 min 0 s"));
    grab("soaking");
    check("QD28: the rest of the soak", dspi_core_vreg_demo_event(b->core(), 3, kSoakS) == 0);
    pollNow();
    check("QD28: passed, and nothing saved by itself", at("passed") && shows("vregCommit") && shows("vregEnd") && deviceInt("storedSel") == 0);
    grab("passed");
    check("QD28: a hang after the pass", dspi_core_vreg_demo_event(b->core(), 0, 0) == 0);
    pollNow();
    check("QD28: failed", at("failed") && vreg->view().value(QStringLiteral("failedVreg")).toInt() == k120
                              && vreg->view().value(QStringLiteral("nextVreg")).toInt() == k125);
    check("QD28: the failure on the page", value("vregFailure").startsWith(QLatin1String("1.20 V")));
    printf("smoke: QD28 after the failure: start shows %d, title \"%s\", voltage %d, next %d\n", shows("vregStart"), qPrintable(title("vregStart")),
           item("vregStartVoltage") ? item("vregStartVoltage")->property("value").toInt() : -1, vreg->view().value(QStringLiteral("nextVreg")).toInt());
    check("QD28: the next step up is offered", shows("vregStart") && title("vregStart") == QLatin1String("Try the next step up")
                                                   && item("vregStartVoltage")->property("value").toInt() == k125);
    grab("failed");
    check("QD28: 1.25 V asks for the word", press("vregStart") && vreg->pending().value(QStringLiteral("needsWord")).toBool() && shows("vregWord"));
    check("QD28: no word, no trial", press("vregCancel") && device().value(QStringLiteral("trialActive")).toBool() == false);
    check("QD28: staged again", press("vregStart"));
    if (QQuickItem *w = item("vregWord")) w->setProperty("text", QStringLiteral("OVERVOLT"));
    check("QD28: 1.25 V with the word", press("vregConfirmButton") && waitFor(1000, idle) && at("clearing"));
    pollNow();
    pollNow();
    check("QD28: soaking at 1.25 V", at("soaking") && deviceInt("trialVreg") == k125);
    check("QD28: a hang at the cap", dspi_core_vreg_demo_event(b->core(), 0, 0) == 0);
    pollNow();
    check("QD28: failed at the cap: a hardware problem", at("failed") && vreg->view().value(QStringLiteral("hardwareProblem")).toBool()
                                                             && title("vregMessage").contains(QLatin1String("hardware problem")));
    grab("hardware-problem");

    // A power cycle mid-soak: unknown, run it again
    if ((page = item("coreVoltagePage"))) page->setProperty("startCode", k120);
    check("QD28: run again at 1.20 V", press("vregStart") && press("vregConfirmButton") && waitFor(1000, idle));
    pollNow();
    pollNow();
    check("QD28: soaking again", at("soaking"));
    check("QD28: a power cycle", dspi_core_vreg_demo_event(b->core(), 2, 0) == 0);
    pollNow();
    check("QD28: unknown", at("unknown") && shows("vregStart") && title("vregStart") == QLatin1String("Run it again"));
    grab("unknown");
    // An abort mid-soak
    check("QD28: run again", press("vregStart") && press("vregConfirmButton") && waitFor(1000, idle));
    pollNow();
    pollNow();
    check("QD28: soaking to abort", at("soaking") && title("vregEnd") == QLatin1String("Abort"));
    check("QD28: Abort goes at once", press("vregEnd") && waitFor(1000, idle) && !device().value(QStringLiteral("trialActive")).toBool());
    pollNow();
    check("QD28: aborted", at("ended") && deviceInt("storedSel") == 0);
    // A pass, then the save
    // Ended offers no start: close, and Calibrate again
    check("QD28: ended offers no start", !shows("vregStart") && shows("vregClose"));
    check("QD28: close and reopen", press("vregClose") && waitFor(1000, idle) && press("vregOpen") && waitFor(1000, idle) && at("review"));
    check("QD28: once more", press("vregStart") && press("vregConfirmButton") && waitFor(1000, idle));
    pollNow();
    pollNow();
    dspi_core_vreg_demo_event(b->core(), 3, kSoakS);
    pollNow();
    check("QD28: passed again", at("passed"));
    check("QD28: Save is only staged", press("vregCommit") && confirmShows() && deviceInt("storedSel") == 0);
    check("QD28: saving", press("vregConfirmButton") && waitFor(1000, idle) && at("committing"));
    pollNow();
    check("QD28: saved", at("committed") && deviceInt("storedSel") == k120);

    // A trial outside the flow's range, from the Terminal's raw path: left alone
    check("QD28: a 1.30 V trial from elsewhere", dspi_core_vreg_op(b->core(), 1, k130, "OVERVOLT") == 0);
    pollNow();
    check("QD28: found", at("trial_found"));
    check("QD28: Keep sends nothing", press("vregKeep") && waitFor(1000, idle) && deviceInt("trialVreg") == k130);
    check("QD28: left running", at("trial_left_running") && !shows("vregCommit"));
    grab("left-running");
    check("QD28: ended from the page", press("vregEnd") && waitFor(1000, idle));
    pollNow();
    check("QD28: and over", at("ended") && !device().value(QStringLiteral("trialActive")).toBool());

    // A read-back that never came: the demo device always applies at once, so the
    // core's view stands in for it
    QVariantMap failed = vreg->view();
    failed.insert(QStringLiteral("ok"), true);
    failed.insert(QStringLiteral("phase"), QStringLiteral("verify_failed"));
    failed.insert(QStringLiteral("messageKey"), QStringLiteral("verify_failed"));
    failed.insert(QStringLiteral("message"), QStringLiteral("The device did not read back the voltage asked for. See its status; nothing was retried."));
    vreg->setViewOverride(QJsonDocument(QJsonObject::fromVariantMap(failed)).toJson(QJsonDocument::Compact));
    check("QD28: not confirmed", at("verify_failed") && title("vregMessage").startsWith(QLatin1String("The device did not read back")));
    grab("verify-failed");
    vreg->setViewOverride(QByteArray());
    pollNow();
    check("QD28: back to the core's view", at("ended"));

    if (seen.size() != kAllPhases) {
        QStringList list(seen.begin(), seen.end());
        printf("smoke: QD28 phases seen: %s\n", qPrintable(list.join(QLatin1Char(' '))));
    }
    check("QD28: every phase showed", seen.size() == kAllPhases);

    // ── Leaving, and a change of device ──
    check("QD28: close again", press("vregClose") && waitFor(1000, idle) && at("idle"));
    check("QD28: Back", press("vregBack") && waitFor(1000, [&]() { return !item("coreVoltagePage") && shows("advancedCoreVoltage"); }));
    check("QD28: no reads off the page", !vreg->watching());
    check("QD28: open again", press("advancedCoreVoltageOpen") && waitFor(1000, [&]() { return shows("coreVoltagePage"); }) && vreg->watching());
    navigate("about");
    check("QD28: leaving Advanced closes it", !item("coreVoltagePage") && !vreg->watching());
    navigate("advanced");
    check("QD28: Advanced comes back on its own page", shows("advancedCoreVoltage") && !item("coreVoltagePage"));
    press("advancedCoreVoltageOpen");
    spin(100);
    pollNow();
    check("QD28: a request staged", press("vregClear") && confirmShows());
    b->disconnectDevice();
    check("QD28: a staged request goes with the device", vreg->pending().isEmpty() && !confirmShows());
    check("QD28: nothing to confirm", !vreg->confirm());
    b->rescan();
    check("QD28: a fresh device", b->connected() && waitFor(2000, [&]() { return vreg->polled(); }));
    pollNow();
    check("QD28: starts clean", at("idle") && value("vregStored") == QLatin1String("Build default") && value("vregHangs") == QLatin1String("0"));

    // Firmware without the extension: Advanced offers nothing
    navigate("about");
    ext->setStatusOverride(R"({"ok":true,"supported":[],"hasPairMap":false,"hasCoreVreg":false})");
    navigate("advanced");
    check("QD28: stock: not on Advanced", item("advancedCoreVoltage") && !shows("advancedCoreVoltage"));
    grab("advanced-stock");
    ext->setStatusOverride(QByteArray());
    spin(50);
    check("QD28: back with the extension", shows("advancedCoreVoltage"));
    // Read-only firmware hides it through bridge.writable; QD6's pass covers that page
    navigate("about");

    vreg->setPollPaused(false);
    settings->setVisible(false);
    spin(50);
    b->disconnectDevice();
    b->rescan();
    spin(100);
}
