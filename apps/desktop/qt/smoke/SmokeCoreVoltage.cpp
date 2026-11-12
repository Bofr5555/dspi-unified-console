// -- WP QP23 -- The Core Voltage page (behaviour.md §20) on the demo device:
// reached only from Advanced; every state of the calibration flow, played with
// the core's demo hooks, with a screenshot of each in the dark and the light
// scheme; the OVERVOLT gate (Confirm stays off until the word matches
// exactly); the manual requests off while a calibration runs; names and the
// Tab order; hidden on firmware without the extension. Never on a real device:
// the section stops unless the demo device is open.
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreVoltageController.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "ExtController.h"
#include "SettingsController.h"
#include "Shell.h"

#include "CoreApi.h"

#include <QAccessible>
#include <QGuiApplication>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <functional>

namespace {

struct Page {
    SmokeDriver &d;
    QQuickWindow *w;
    CoreVoltageController *cv;

    QQuickItem *item(const char *name) const { return d.item(QString::fromLatin1(name), w->contentItem()); }
    QQuickItem *page() const { return item("CoreVoltagePage"); }
    bool shown(const char *name) const {
        for (QQuickItem *i = item(name); i; i = i->parentItem())
            if (!i->isVisible()) return false;
        return item(name) != nullptr;
    }
    bool enabled(const char *name) const {
        QQuickItem *i = item(name);
        return i && i->isEnabled();
    }
    QString text(const char *name) const {
        QQuickItem *i = item(name);
        return i ? i->property("text").toString() : QString();
    }
    void click(const char *name) const {
        if (QQuickItem *i = item(name)) QMetaObject::invokeMethod(i, "click");
        d.spin(60);
    }
    bool phaseIs(const char *phase, int ms = 4000) const {
        return d.waitFor(ms, [&]() { return cv->phase() == QLatin1String(phase) && !cv->busy(); });
    }
    // Waits for the requests asked so far to come back
    void settle() const {
        d.waitIdle();
        d.waitFor(2000, [&]() { return !cv->busy(); });
        d.spin(120);
    }
    // settings-core-voltage-<state>.png and its -light twin, scrolled to `section`
    void shoot(const QString &state, const char *section = nullptr) const {
        if (section) QMetaObject::invokeMethod(page(), "reveal", Q_ARG(QVariant, QVariant::fromValue<QObject *>(item(section))));
        Shell *shell = d.app().shell;
        const Qt::ColorScheme before = shell->colorSchemeOverride();
        shell->setColorSchemeOverride(Qt::ColorScheme::Dark);
        d.spin(150);
        d.check(QStringLiteral("qp23: screenshot of %1, dark").arg(state),
                d.grab(w, QStringLiteral("settings-core-voltage-%1.png").arg(state)));
        shell->setColorSchemeOverride(Qt::ColorScheme::Light);
        d.spin(150);
        d.check(QStringLiteral("qp23: screenshot of %1, light").arg(state),
                d.grab(w, QStringLiteral("settings-core-voltage-%1-light.png").arg(state)));
        shell->setColorSchemeOverride(before);
        d.spin(50);
    }
};

const char *const kManual[] = { "CoreVoltage.manualTrial", "CoreVoltage.manualSave", "CoreVoltage.manualReset",
                                "CoreVoltage.manualAuto", "CoreVoltage.manualClear" };

bool manualAllOff(const Page &p) {
    for (const char *name : kManual)
        if (p.enabled(name)) return false;
    return !p.enabled("CoreVoltage.manualVoltage.combo");
}

// QTest::keyClicks takes widgets only: one key at a time into the window
void typeText(QQuickWindow *w, const QString &text) {
    for (const QChar c : text) QTest::keyClick(w, c.toLatin1());
}

void walk(QQuickItem *root, const std::function<void(QQuickItem *)> &fn) {
    if (!root) return;
    fn(root);
    for (QQuickItem *child : root->childItems()) walk(child, fn);
}

bool visibleTree(QQuickItem *item) {
    for (QQuickItem *i = item; i; i = i->parentItem())
        if (!i->isVisible()) return false;
    return item->width() > 0 && item->height() > 0;
}

// Every control Tab reaches on the page has a name; Tab from Back meets the
// page's named controls in reading order: Back, the calibration, Confirm (when
// open), then the manual requests
void checkKeyboard(const Page &p, const QString &state, const QStringList &expected) {
    SmokeDriver &d = p.d;
    QStringList unnamed;
    int controls = 0;
    walk(p.page(), [&](QQuickItem *item) {
        if (!item->activeFocusOnTab() || !item->isEnabled() || !visibleTree(item)) return;
        ++controls;
        QAccessibleInterface *a = QAccessible::queryAccessibleInterface(item);
        if (!a || a->text(QAccessible::Name).trimmed().isEmpty())
            unnamed << (item->objectName().isEmpty() ? QString::fromLatin1(item->metaObject()->className()) : item->objectName());
    });
    if (!unnamed.isEmpty()) std::printf("smoke: qp23: %s: unnamed: %s\n", qPrintable(state), qPrintable(unnamed.join(u", ")));
    d.check(QStringLiteral("qp23: %1: every control Tab reaches has a name (%2)").arg(state).arg(controls), unnamed.isEmpty() && controls > 0);

    p.w->requestActivate();
    d.waitFor(1500, [&]() { return QGuiApplication::focusWindow() == p.w; });
    QQuickItem *back = p.item("CoreVoltage.back");
    if (!back || QGuiApplication::focusWindow() != p.w) {
        d.notRun(QStringLiteral("qp23: %1: the Tab order").arg(state), QStringLiteral("the window did not take the keyboard"));
        return;
    }
    back->forceActiveFocus(Qt::TabFocusReason);
    QStringList order{ back->objectName() };
    for (int i = 0; i < 40; ++i) {
        QTest::keyClick(p.w, Qt::Key_Tab);
        QQuickItem *now = p.w->activeFocusItem();
        // A combo box's own name is its picker row's plus ".combo"
        QString name;
        for (QQuickItem *it = now; it && name.isEmpty(); it = it->parentItem())
            if (it->objectName().startsWith(QLatin1String("CoreVoltage."))) name = it->objectName();
        if (name == back->objectName()) break;
        if (!name.isEmpty() && order.constLast() != name) order << name;
    }
    std::printf("smoke: qp23: %s: Tab order: %s\n", qPrintable(state), qPrintable(order.join(u", ")));
    QStringList met;
    for (const QString &n : order)
        if (expected.contains(n)) met << n;
    d.check(QStringLiteral("qp23: %1: Tab walks the page in reading order").arg(state), met == expected);
}

bool isDemo(DeviceModel *device) {
    // The built-in demo (dspi_open_demo: the core's simulated device); a real
    // device never gets here
    return device->connected() && device->demo();
}

void coreVoltageSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *settings = Registrar::instance<SettingsController>();
    auto *cv = Registrar::instance<CoreVoltageController>();
    auto *ext = Registrar::instance<ExtController>();
    d.check(QStringLiteral("qp23: the controllers exist"), settings && cv && ext);
    if (!settings || !cv || !ext) return;
    if (!d.waitFor(5000, [&]() { return device->connected(); }) || !isDemo(device)) {
        // Never a voltage on anything but the demo device (the owner's rule)
        d.notRun(QStringLiteral("qp23: the core voltage page"), QStringLiteral("the demo device is not open"));
        return;
    }
    d.check(QStringLiteral("qp23: the demo device has the extension"),
            d.waitFor(4000, [&]() { return cv->supported() && cv->view().value(QStringLiteral("available")).toBool(); }));

    // ---- Only from Advanced ------------------------------------------------------------
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check(QStringLiteral("qp23: the settings window"), w && d.waitFor(3000, [&]() { return w->isVisible(); }));
    if (!w) return;
    Page p{ d, w, cv };
    bool inSidebar = false;
    for (const QVariant &row : settings->sidebarRows(QString()))
        inSidebar |= row.toMap().value(QStringLiteral("id")).toString() == QLatin1String("coreVoltage");
    d.check(QStringLiteral("qp23: the page is not in the sidebar"), !inSidebar);
    settings->open(QStringLiteral("advanced"));
    d.spin(300);
    d.check(QStringLiteral("qp23: Advanced shows the entry"),
            d.waitFor(3000, [&]() { return p.shown("Advanced.coreVoltage") && p.shown("Advanced.coreVoltageOpen"); }));
    p.click("Advanced.coreVoltageOpen");
    d.check(QStringLiteral("qp23: Open… opens the page"),
            d.waitFor(2000, [&]() { return settings->currentPage() == QLatin1String("coreVoltage") && p.page(); }));
    d.check(QStringLiteral("qp23: the page polls while it shows"), cv->pageShown());
    p.settle();
    const QVariantMap view = cv->view();
    d.check(QStringLiteral("qp23: the warning is the core's, word for word"),
            !view.value(QStringLiteral("warning")).toString().isEmpty() &&
                p.text("CoreVoltage.warningText") == view.value(QStringLiteral("warning")).toString());
    d.check(QStringLiteral("qp23: the device's rows show"),
            d.waitFor(3000, [&]() { return p.shown("CoreVoltage.running") && p.text("CoreVoltage.running.value").endsWith(QLatin1String(" V")); }) &&
                p.shown("CoreVoltage.saved") && p.shown("CoreVoltage.default") && p.shown("CoreVoltage.hangs") &&
                p.shown("CoreVoltage.lastFailed"));
    const QString word = view.value(QStringLiteral("confirmationWord")).toString();
    d.check(QStringLiteral("qp23: the core names its word"), !word.isEmpty());

    // ---- Idle ------------------------------------------------------------------------
    // A flow left from earlier ends here: the demo starts from nothing
    if (cv->phase() != QLatin1String("idle")) {
        cv->endTrial();
        p.settle();
        cv->closeFlow();
        p.settle();
    }
    d.check(QStringLiteral("qp23: idle: Not started"), p.phaseIs("idle") && p.text("CoreVoltage.state.value") == cv->stateName(QStringLiteral("idle")));
    d.check(QStringLiteral("qp23: idle: Calibrate… and the manual requests are on"),
            p.enabled("CoreVoltage.begin") && p.enabled("CoreVoltage.manualTrial") && p.enabled("CoreVoltage.manualClear"));
    p.shoot(QStringLiteral("idle"));
    checkKeyboard(p, QStringLiteral("idle"),
                  { QStringLiteral("CoreVoltage.back"), QStringLiteral("CoreVoltage.begin"),
                    QStringLiteral("CoreVoltage.manualVoltage.combo"), QStringLiteral("CoreVoltage.manualTrial"),
                    QStringLiteral("CoreVoltage.manualSave"), QStringLiteral("CoreVoltage.manualReset"),
                    QStringLiteral("CoreVoltage.manualAuto"), QStringLiteral("CoreVoltage.manualClear") });

    // ---- Review --------------------------------------------------------------------
    p.click("CoreVoltage.begin");
    d.check(QStringLiteral("qp23: Calibrate… opens the review"), p.phaseIs("review") && p.text("CoreVoltage.state.value") == cv->stateName(QStringLiteral("review")));
    d.check(QStringLiteral("qp23: review: the description is the core's"),
            p.text("CoreVoltage.message") == cv->view().value(QStringLiteral("message")).toString() && !p.text("CoreVoltage.message").isEmpty());
    d.check(QStringLiteral("qp23: review: the voltage and soak pickers and Start…"),
            p.shown("CoreVoltage.trialVoltage") && p.shown("CoreVoltage.soakTime") && p.enabled("CoreVoltage.start"));
    const QVariantList choices = cv->view().value(QStringLiteral("choices")).toList();
    QObject *soakCombo = p.item("CoreVoltage.soakTime.combo");
    const int suggested = cv->view().value(QStringLiteral("defaultSoakS")).toInt();
    d.check(QStringLiteral("qp23: review: the core's suggested soak is picked and marked"),
            soakCombo && cv->soakChoices().indexOf(suggested) == soakCombo->property("currentIndex").toInt() &&
                soakCombo->property("currentText").toString().contains(cv->durationLabel(suggested)) &&
                soakCombo->property("currentText").toString() != cv->durationLabel(suggested));
    d.check(QStringLiteral("qp23: review: no soak under the core's minimum is offered"),
            !cv->soakChoices().isEmpty() &&
                cv->soakChoices().constFirst().toInt() == cv->view().value(QStringLiteral("minSoakS")).toInt());
    p.shoot(QStringLiteral("review"), "CoreVoltage.calibrate");
    checkKeyboard(p, QStringLiteral("review"),
                  { QStringLiteral("CoreVoltage.back"), QStringLiteral("CoreVoltage.trialVoltage.combo"),
                    QStringLiteral("CoreVoltage.soakTime.combo"), QStringLiteral("CoreVoltage.start"),
                    QStringLiteral("CoreVoltage.close"), QStringLiteral("CoreVoltage.manualVoltage.combo") });

    // ---- The confirm word: a voltage the core asks the word for ---------------------
    int high = -1, plain = -1;
    for (const QVariant &c : choices) {
        const QVariantMap m = c.toMap();
        if (m.value(QStringLiteral("needsWord")).toBool() && high < 0) high = m.value(QStringLiteral("code")).toInt();
        if (!m.value(QStringLiteral("needsWord")).toBool() && plain < 0) plain = m.value(QStringLiteral("code")).toInt();
    }
    d.check(QStringLiteral("qp23: the demo offers a voltage with and one without the word"), high > 0 && plain > 0);
    p.page()->setProperty("trialCode", high);
    d.spin(50);
    const int sent = cv->requests();
    p.click("CoreVoltage.start");
    d.check(QStringLiteral("qp23: Start… opens Confirm and sends nothing"),
            p.shown("CoreVoltage.confirm") && cv->requests() == sent && cv->phase() == QLatin1String("review"));
    d.check(QStringLiteral("qp23: Confirm asks for the core's word"),
            p.shown("CoreVoltage.word") && p.text("CoreVoltage.wordPrompt").contains(word));
    d.check(QStringLiteral("qp23: confirm word: Confirm is off with nothing typed"), !p.enabled("CoreVoltage.confirmButton"));
    d.check(QStringLiteral("qp23: confirm word: the other requests wait"),
            !p.enabled("CoreVoltage.start") && manualAllOff(p));
    QQuickItem *field = p.item("CoreVoltage.word");
    bool gate = true;
    for (const QString &near : { word.toLower(), word.left(word.size() - 1), word + QLatin1Char(' '), QLatin1Char(' ') + word,
                                 word + word.right(1), QString(word).replace(0, 1, word.at(0).toLower()) }) {
        field->setProperty("text", near);
        d.spin(20);
        if (p.enabled("CoreVoltage.confirmButton")) {
            std::printf("smoke: qp23: Confirm is on for \"%s\"\n", qPrintable(near));
            gate = false;
        }
        p.click("CoreVoltage.confirmButton");
        QTest::keyClick(w, Qt::Key_Return);
    }
    d.check(QStringLiteral("qp23: confirm word: Confirm stays off for every near miss, and nothing is sent"),
            gate && cv->requests() == sent && cv->phase() == QLatin1String("review"));
    field->setProperty("text", QString());
    field->forceActiveFocus();
    typeText(w, word);
    d.spin(50);
    d.check(QStringLiteral("qp23: confirm word: typed exactly, Confirm comes on"), p.enabled("CoreVoltage.confirmButton"));
    p.shoot(QStringLiteral("confirm-word"), "CoreVoltage.confirm");
    checkKeyboard(p, QStringLiteral("confirm-word"),
                  { QStringLiteral("CoreVoltage.back"), QStringLiteral("CoreVoltage.word"),
                    QStringLiteral("CoreVoltage.confirmButton"), QStringLiteral("CoreVoltage.cancel") });
    p.click("CoreVoltage.cancel");
    d.check(QStringLiteral("qp23: Cancel leaves, nothing sent"),
            !p.shown("CoreVoltage.confirm") && cv->requests() == sent && cv->phase() == QLatin1String("review"));

    // ---- Soaking -------------------------------------------------------------------
    p.page()->setProperty("trialCode", plain);
    d.spin(50);
    p.click("CoreVoltage.start");
    d.check(QStringLiteral("qp23: without the word, Confirm is on at once"),
            p.shown("CoreVoltage.confirm") && !p.shown("CoreVoltage.word") && p.enabled("CoreVoltage.confirmButton"));
    p.click("CoreVoltage.confirmButton");
    d.check(QStringLiteral("qp23: Confirm starts the trial: soaking"), p.phaseIs("soaking", 6000));
    d.check(QStringLiteral("qp23: soaking: the elapsed and total time, Abort in red"),
            p.shown("CoreVoltage.progress") && p.text("CoreVoltage.progress").contains(cv->durationLabel(suggested)) &&
                p.shown("CoreVoltage.abort") && p.item("CoreVoltage.abort")->property("destructive").toBool());
    d.check(QStringLiteral("qp23: soaking: Running at says (trial)"), p.text("CoreVoltage.running.value").contains(QLatin1String("(")));
    d.check(QStringLiteral("qp23: soaking: the manual requests are off"), cv->calibrating() && manualAllOff(p));
    const int beforeManual = cv->requests();
    for (const char *name : kManual) p.click(name);
    d.check(QStringLiteral("qp23: soaking: clicking them asks nothing"), cv->requests() == beforeManual && !p.shown("CoreVoltage.confirm"));
    cv->demoEvent(3, 120);
    p.settle();
    d.check(QStringLiteral("qp23: soaking: the elapsed time counts"), cv->view().value(QStringLiteral("elapsedS")).toInt() >= 120);
    p.shoot(QStringLiteral("soaking"), "CoreVoltage.calibrate");

    // ---- Passed, then committed -----------------------------------------------------
    cv->demoEvent(3, uint32_t(suggested));
    d.check(QStringLiteral("qp23: the soak runs out: passed"), p.phaseIs("passed", 5000));
    d.check(QStringLiteral("qp23: passed: Save… and End Trial; the manual requests stay off"),
            p.enabled("CoreVoltage.commit") && p.enabled("CoreVoltage.endTrial") && manualAllOff(p));
    p.shoot(QStringLiteral("passed"), "CoreVoltage.calibrate");
    p.click("CoreVoltage.commit");
    d.check(QStringLiteral("qp23: Save… asks first"), p.shown("CoreVoltage.confirm") && cv->phase() == QLatin1String("passed"));
    if (p.shown("CoreVoltage.word")) {
        field->forceActiveFocus();
        typeText(w, word);
    }
    p.click("CoreVoltage.confirmButton");
    d.check(QStringLiteral("qp23: committed, read back"), p.phaseIs("committed", 8000));
    d.check(QStringLiteral("qp23: committed: the saved voltage is the trial's"),
            d.waitFor(3000, [&]() { return cv->status().value(QStringLiteral("storedSel")).toInt() == plain; }));
    p.shoot(QStringLiteral("committed"), "CoreVoltage.calibrate");
    p.click("CoreVoltage.close");
    d.check(QStringLiteral("qp23: Close goes back to idle"), p.phaseIs("idle"));

    // ---- Failed --------------------------------------------------------------------
    p.click("CoreVoltage.begin");
    p.phaseIs("review");
    p.page()->setProperty("trialCode", plain);
    p.click("CoreVoltage.start");
    p.click("CoreVoltage.confirmButton");
    p.phaseIs("soaking", 6000);
    cv->demoEvent(3, 60);
    p.settle();
    cv->demoEvent(0, 0);
    d.check(QStringLiteral("qp23: a hang during the soak: failed"), p.phaseIs("failed", 6000));
    d.check(QStringLiteral("qp23: failed: the core's description, and the hang on record"),
            p.text("CoreVoltage.message") == cv->view().value(QStringLiteral("message")).toString() &&
                cv->view().value(QStringLiteral("failedVreg")).toInt() == plain &&
                p.text("CoreVoltage.lastFailed.value") != QObject::tr("None"));
    d.check(QStringLiteral("qp23: failed: the manual requests come back"), !cv->calibrating() && p.enabled("CoreVoltage.manualClear"));
    p.shoot(QStringLiteral("failed"), "CoreVoltage.calibrate");

    // ---- Failed at the top of the range: a hardware problem -------------------------
    p.page()->setProperty("trialCode", high);
    d.spin(50);
    p.click("CoreVoltage.start");
    d.check(QStringLiteral("qp23: a retry above the line asks for the word again"),
            p.shown("CoreVoltage.word") && !p.enabled("CoreVoltage.confirmButton"));
    field->forceActiveFocus();
    typeText(w, word);
    p.click("CoreVoltage.confirmButton");
    p.phaseIs("soaking", 6000);
    cv->demoEvent(3, 30);
    p.settle();
    cv->demoEvent(0, 0);
    d.check(QStringLiteral("qp23: a hang at the top: failed, a hardware problem in the core's words"),
            p.phaseIs("failed", 6000) && cv->view().value(QStringLiteral("hardwareProblem")).toBool() &&
                p.text("CoreVoltage.message") == cv->view().value(QStringLiteral("message")).toString());
    p.shoot(QStringLiteral("hardware-problem"), "CoreVoltage.calibrate");
    p.click("CoreVoltage.close");
    p.phaseIs("idle");

    // ---- Unknown: the power goes during a trial -------------------------------------
    p.click("CoreVoltage.begin");
    p.phaseIs("review");
    p.page()->setProperty("trialCode", plain);
    p.click("CoreVoltage.start");
    p.click("CoreVoltage.confirmButton");
    p.phaseIs("soaking", 6000);
    cv->demoEvent(2, 0);
    d.check(QStringLiteral("qp23: a power cycle during the soak: unknown, in the core's words"),
            p.phaseIs("unknown", 6000) && p.text("CoreVoltage.message") == cv->view().value(QStringLiteral("message")).toString());
    p.shoot(QStringLiteral("unknown"), "CoreVoltage.calibrate");
    p.click("CoreVoltage.close");
    p.phaseIs("idle");

    // ---- Trial found: a manual trial above the word's line, word typed --------------
    p.page()->setProperty("manualCode", high);
    d.spin(50);
    p.click("CoreVoltage.manualTrial");
    d.check(QStringLiteral("qp23: Trial… above the line asks for the word"),
            p.shown("CoreVoltage.word") && !p.enabled("CoreVoltage.confirmButton"));
    field->forceActiveFocus();
    typeText(w, word);
    QTest::keyClick(w, Qt::Key_Return);
    d.check(QStringLiteral("qp23: the trial runs: trial found"), p.phaseIs("trial_found", 6000));
    d.check(QStringLiteral("qp23: trial found: End Trial and Keep…"),
            p.enabled("CoreVoltage.endTrial") && p.shown("CoreVoltage.keep"));
    p.shoot(QStringLiteral("trial-found"), "CoreVoltage.calibrate");
    p.click("CoreVoltage.endTrial");
    d.check(QStringLiteral("qp23: End Trial ends it"),
            d.waitFor(6000, [&]() { return !cv->status().value(QStringLiteral("trialActive")).toBool() && !cv->busy(); }));
    if (cv->phase() != QLatin1String("idle")) {
        p.click("CoreVoltage.close");
        p.phaseIs("idle");
    }

    // ---- Back to the build default, as the demo began --------------------------------
    p.click("CoreVoltage.manualReset");
    if (p.shown("CoreVoltage.word")) {
        field->forceActiveFocus();
        typeText(w, word);
    }
    p.click("CoreVoltage.confirmButton");
    d.check(QStringLiteral("qp23: Reset… saves the build default"),
            d.waitFor(6000, [&]() { return cv->status().value(QStringLiteral("storedSel")).toInt() == 0 && !cv->busy(); }));
    if (cv->phase() != QLatin1String("idle")) {
        p.click("CoreVoltage.close");
        p.phaseIs("idle");
    }
    p.click("CoreVoltage.manualClear");
    p.click("CoreVoltage.confirmButton");
    d.check(QStringLiteral("qp23: Clear… forgets the failure"),
            d.waitFor(4000, [&]() { return cv->status().value(QStringLiteral("failedTrialVreg")).toInt() == 0 && !cv->busy(); }));

    // ---- Back --------------------------------------------------------------------
    p.click("CoreVoltage.back");
    d.check(QStringLiteral("qp23: Back returns to Advanced and stops the poll"),
            d.waitFor(2000, [&]() { return settings->currentPage() == QLatin1String("advanced") && !cv->pageShown(); }));

    // ---- Stock firmware: no extension, no entry, no page -----------------------------
    ext->setStatusOverride(R"({"ok":true,"supported":[],"hasPairMap":false,"hasCoreVreg":false})");
    d.check(QStringLiteral("qp23: without the extension the entry hides"),
            d.waitFor(3000, [&]() { return !cv->supported() && !p.shown("Advanced.coreVoltage"); }));
    d.check(QStringLiteral("qp23: and the page is unavailable"), !settings->isShown(QStringLiteral("coreVoltage")));
    d.grab(w, QStringLiteral("settings-core-voltage-stock.png"));
    ext->setStatusOverride(QByteArray());
    d.check(QStringLiteral("qp23: the entry comes back with the probe"),
            d.waitFor(3000, [&]() { return cv->supported() && p.shown("Advanced.coreVoltage"); }));
    w->close();
    d.spin(50);
}

const SmokeDriver::Section coreVoltage("qp23-vreg", 100, &coreVoltageSection);

} // namespace
