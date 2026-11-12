// -- WP QP17 -- Onboarding and the Help menu: the core's words reach the
// Getting Started wizard and What's New resolved for this platform; the wizard
// replaces the console and goes for good on Skip or at the end; every board
// state has its card and header (through a stand-in for the firmware
// package's installer); the release notes' launch rule; the About licence
// block and the developer rows on Advanced; Help's rows. Screenshots of each.
#include "SmokeDriver.h"

#include "AboutController.h"
#include "OnboardingController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"

#include <QGuiApplication>
#include <QJsonDocument>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <cstdio>
#include <functional>

namespace {

// Stands in for QP16's FirmwareInstaller, with what the board step reads
class QP17Installer : public QObject {
    Q_OBJECT
    Q_PROPERTY(QString state MEMBER state NOTIFY changed)
    Q_PROPERTY(QString chip MEMBER chip CONSTANT)
    Q_PROPERTY(QString volume MEMBER volume CONSTANT)
    Q_PROPERTY(QString version MEMBER version CONSTANT)
    Q_PROPERTY(QString failure MEMBER failure CONSTANT)
    Q_PROPERTY(double fraction MEMBER fraction CONSTANT)
    Q_PROPERTY(bool mundane MEMBER mundane NOTIFY changed)
public:
    QString state = QStringLiteral("idle");
    QString chip = QStringLiteral("RP2350");
    QString volume = QStringLiteral("RP2350");
    QString version = QStringLiteral("1.1.6");
    QString failure = QStringLiteral("The board went away.");
    double fraction = 0.4;
    bool mundane = false;
    int watching = 0, armed = 0, connected = 0, resets = 0;
    void set(const QString &s, bool m = false) {
        state = s;
        mundane = m;
        emit changed();
    }
    Q_INVOKABLE void beginWatching() { ++watching; }
    Q_INVOKABLE void stopWatching() { --watching; }
    Q_INVOKABLE void installWhenReady() { ++armed; }
    Q_INVOKABLE void installConnected() { ++connected; }
    Q_INVOKABLE void reset() { ++resets; }
signals:
    void changed();
};

QObject *qp17Item(QObject *root, const QString &name) {
    if (!root) return nullptr;
    if (root->objectName() == name) return root;
    if (auto *item = qobject_cast<QQuickItem *>(root))
        for (QQuickItem *child : item->childItems())
            if (QObject *o = qp17Item(child, name)) return o;
    for (QObject *child : root->children())
        if (QObject *o = qp17Item(child, name)) return o;
    return nullptr;
}

// The texts of every visible item under `from`
void qp17Texts(QQuickItem *from, QStringList *out) {
    if (!from || !from->isVisible()) return;
    const QVariant t = from->property("text");
    if (t.isValid() && !t.toString().isEmpty()) out->append(t.toString());
    for (QQuickItem *c : from->childItems()) qp17Texts(c, out);
}

// Nothing of the assets' syntax or a hole left
bool qp17Resolved(const QStringList &texts) {
    for (const QString &t : texts)
        if (t.contains(QLatin1String("[[")) || t.contains(QLatin1String("]]")) || t.contains(QLatin1Char('{'))) {
            std::printf("smoke:   unresolved: %s\n", qPrintable(t));
            return false;
        }
    return true;
}

QVariantMap qp17Stage(const QVariantMap &words, const char *id) {
    for (const QVariant &s : words.value(QStringLiteral("stages")).toList())
        if (s.toMap().value(QStringLiteral("id")).toString() == QLatin1String(id)) return s.toMap();
    return {};
}

// Every `text` (or plain string) of `list` is among `shown`
bool qp17AllShown(const QVariantList &list, const QStringList &shown) {
    if (list.isEmpty()) return false;
    for (const QVariant &v : list) {
        const QString t = v.typeId() == QMetaType::QString ? v.toString() : v.toMap().value(QStringLiteral("text")).toString();
        if (t.isEmpty() || !shown.contains(t)) {
            std::printf("smoke:   not shown: %s\n", qPrintable(t));
            return false;
        }
    }
    return true;
}

void onboardingSection(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    auto *ob = Registrar::instance<OnboardingController>();
    auto *settings = Registrar::instance<SettingsController>();
    d.check(QStringLiteral("qp17: the onboarding controller is registered"), ob && settings);
    if (!ob || !settings) return;

    // ---- The core's data --------------------------------------------------
    const QString notesJson = QString::fromUtf8(QJsonDocument::fromVariant(ob->releases()).toJson());
    d.check(QStringLiteral("qp17: release notes come from the core"), !ob->releases().isEmpty() && notesJson.contains("1.1.6-beta4"));
    d.check(QStringLiteral("qp17: release notes have nothing left to resolve"), !notesJson.contains(QLatin1String("[[")));
    d.check(QStringLiteral("qp17: the notes name this app's menu"),
            notesJson.contains(QLatin1String("Effects > ")) && !notesJson.contains(QLatin1String("Tools > Tube")));
    d.check(QStringLiteral("qp17: the notes are newest first"),
            ob->releases().value(0).toMap().value(QStringLiteral("version")).toString() == "1.1.6");
    d.check(QStringLiteral("qp17: the bundled firmware is read from the images"), ob->bundledFirmware() == "1.1.6");
    d.check(QStringLiteral("qp17: firmware versions compare as the Mac's"),
            ob->firmwareMatch(QStringLiteral("1.1.6")) == "match" && ob->firmwareMatch(QStringLiteral("1.1.5")) == "older"
                && ob->firmwareMatch(QStringLiteral("1.1.6-beta5")) == "older"
                && ob->firmwareMatch(QStringLiteral("1.2.0")) == "newer" && ob->firmwareMatch(QString()).isEmpty());

    // ---- Help's rows ------------------------------------------------------
    d.check(QStringLiteral("qp17: Help's rows are handled"),
            shell->handles(QStringLiteral("help.gettingStarted")) && shell->handles(QStringLiteral("help.whatsNew")));
    QObject *diagRow = d.object(QStringLiteral("Menu.help.saveDiagnostics"));
    d.check(QStringLiteral("qp17: Save Diagnostics waits for the core's bundle"),
            diagRow && (ob->diagnosticsAvailable() == shell->handles(QStringLiteral("help.saveDiagnostics")))
                && diagRow->property("visible").toBool() == ob->diagnosticsAvailable());
    std::printf("smoke: qp17 diagnostics in this core: %s\n", ob->diagnosticsAvailable() ? "yes" : "no");

    // ---- What's New: the launch rule, the window --------------------------
    d.check(QStringLiteral("qp17: a new install starts caught up on the notes"), ob->whatsNewShown() == ob->bundledFirmware());
    d.check(QStringLiteral("qp17: unread notes: none for a new install or the current version"),
            ob->unreadSince(QString()).isEmpty() && ob->unreadSince(QStringLiteral("1.1.6")).isEmpty());
    d.check(QStringLiteral("qp17: unread notes after an update from 1.1.6-beta4"),
            ob->unreadSince(QStringLiteral("1.1.6-beta4")).size() == 2);
    QSettings().setValue(QStringLiteral("onboarding/whatsNewShown"), QStringLiteral("1.1.6-beta4"));
    ob->showReleaseNotesIfUnread();
    d.spin(400);
    QQuickWindow *notes = nullptr;
    for (QWindow *top : QGuiApplication::topLevelWindows())
        if (top->objectName() == QLatin1String("WhatsNewWindow")) notes = qobject_cast<QQuickWindow *>(top);
    d.check(QStringLiteral("qp17: after an update What's New opens by itself"),
            notes && notes->isVisible() && ob->whatsNewShown() == ob->bundledFirmware());
    if (notes) {
        // A list view makes the rows it shows; its count is every release
        QObject *list = qp17Item(notes->contentItem(), QStringLiteral("WhatsNew.list"));
        const int releases = list ? list->property("count").toInt() : 0;
        QStringList shown;
        qp17Texts(notes->contentItem(), &shown);
        d.check(QStringLiteral("qp17: What's New lists every release"), releases == ob->releases().size());
        d.check(QStringLiteral("qp17: with the core's headlines and items"),
                shown.contains(ob->releases().value(0).toMap().value(QStringLiteral("headline")).toString())
                    && qp17AllShown(ob->releases().value(0).toMap().value(QStringLiteral("items")).toList(), shown)
                    && qp17Resolved(shown));
        d.check(QStringLiteral("qp17: What's New has the Mac's size"), notes->width() == 460 && notes->height() == 420);
        d.grab(notes, QStringLiteral("window-whatsNew.png"));
        if (QObject *done = qp17Item(notes->contentItem(), QStringLiteral("WhatsNew.done"))) QMetaObject::invokeMethod(done, "click");
        d.spin(200);
        d.check(QStringLiteral("qp17: Done closes it"), !notes->isVisible());
    }

    // ---- The wizard -------------------------------------------------------
    // The wizard is the item of the main window's loader
    QObject *loader = d.object(QStringLiteral("MainWindow.wizard"));
    auto wizardNow = [&]() -> QObject * { return loader ? loader->property("item").value<QObject *>() : nullptr; };
    d.check(QStringLiteral("qp17: setup done: the console shows, not the wizard"),
            ob->setupDone() && !ob->wizardShown() && loader && !wizardNow());
    shell->trigger(QStringLiteral("help.gettingStarted"));
    d.spin(300);
    QObject *wizard = wizardNow();
    QObject *pane = d.object(QStringLiteral("RightPane"));
    d.check(QStringLiteral("qp17: Help > Getting Started replaces the console"),
            ob->wizardShown() && wizard && wizard->property("visible").toBool() && pane && !qobject_cast<QQuickItem *>(pane)->isVisible());
    if (!wizard) return;
    auto *wizardItem = qobject_cast<QQuickItem *>(wizard);
    const QVariantMap words = ob->gettingStarted();
    auto shownNow = [&]() {
        QStringList shown;
        qp17Texts(wizardItem, &shown);
        return shown;
    };
    auto click = [&](const char *name) {
        if (QObject *o = qp17Item(wizard, QString::fromLatin1(name))) QMetaObject::invokeMethod(o, "click");
        d.spin(150);
    };
    const QVariantMap buttons = words.value(QStringLiteral("buttons")).toMap();

    QStringList shown = shownNow();
    const QVariantMap welcome = qp17Stage(words, "welcome");
    d.check(QStringLiteral("qp17: Welcome shows the core's title, paragraphs and rows"),
            shown.contains(welcome.value(QStringLiteral("title")).toString())
                && qp17AllShown(welcome.value(QStringLiteral("paragraphs")).toList(), shown)
                && qp17AllShown(welcome.value(QStringLiteral("rows")).toList(), shown));
    d.check(QStringLiteral("qp17: Welcome is resolved, the counter filled in"),
            qp17Resolved(shown) && shown.contains(QStringLiteral("Step 1 of 3")));
    d.check(QStringLiteral("qp17: the wizard's buttons have the core's words"),
            shown.contains(buttons.value(QStringLiteral("skip")).toString())
                && shown.contains(buttons.value(QStringLiteral("continue")).toString()));
    d.grab(d.mainWindow(), QStringLiteral("window-wizard.png"));

    click("GettingStarted.continue");
    d.check(QStringLiteral("qp17: Continue goes to the board step"), wizard->property("current").toInt() == 1);
    shown = shownNow();
    d.check(QStringLiteral("qp17: the board step for the connected demo is resolved"),
            qp17Resolved(shown) && !qp17Item(wizard, QStringLiteral("GettingStarted.title"))->property("text").toString().isEmpty());
    d.grab(d.mainWindow(), QStringLiteral("window-wizard-board.png"));

    // Every state of the installer has its header and card, all resolved
    QP17Installer installer;
    ob->setInstaller(&installer);
    d.spin(100);
    d.check(QStringLiteral("qp17: the wizard watches through the installer"), installer.watching > 0);
    bool allWords = true;
    for (const char *state : { "idle", "waitingForBoard", "waitingForVolume", "ready", "writing", "waitingForDevice",
                               "verified", "failed", "checking", "question", "inBootloader", "unconfirmed" }) {
        for (bool mundane : { false, true }) {
            installer.set(QString::fromLatin1(state), mundane);
            d.spin(30);
            const QString title = qp17Item(wizard, QStringLiteral("GettingStarted.title"))->property("text").toString();
            const QString cardTitle = qp17Item(wizard, QStringLiteral("GettingStarted.cardTitle"))->property("text").toString();
            const QString cardText = qp17Item(wizard, QStringLiteral("GettingStarted.cardText"))->property("text").toString();
            if (title.isEmpty() || cardTitle.isEmpty() || cardText.isEmpty() || !qp17Resolved({ title, cardTitle, cardText })) {
                std::printf("smoke:   board state %s: '%s' / '%s' / '%s'\n", state, qPrintable(title), qPrintable(cardTitle),
                            qPrintable(cardText));
                allWords = false;
            }
        }
    }
    d.check(QStringLiteral("qp17: every board state has its header and card"), allWords);
    installer.set(QStringLiteral("writing"));
    d.spin(50);
    d.check(QStringLiteral("qp17: writing shows no Back and no Continue"),
            !qp17Item(wizard, QStringLiteral("GettingStarted.back"))->property("visible").toBool()
                && !qp17Item(wizard, QStringLiteral("GettingStarted.continue"))->property("visible").toBool());
    d.grab(d.mainWindow(), QStringLiteral("window-wizard-writing.png"));
    installer.set(QStringLiteral("ready"));
    d.spin(50);
    click("GettingStarted.cardAction");
    QString preparing;
    for (const QVariant &c : qp17Stage(words, "board").value(QStringLiteral("cards")).toList())
        if (c.toMap().value(QStringLiteral("id")).toString() == QLatin1String("card.preparing"))
            preparing = c.toMap().value(QStringLiteral("title")).toString();
    d.check(QStringLiteral("qp17: Install arms the installer once and says it prepares"),
            installer.armed == 1 && !preparing.isEmpty()
                && qp17Item(wizard, QStringLiteral("GettingStarted.cardTitle"))->property("text").toString() == preparing);
    installer.set(QStringLiteral("failed"));
    d.spin(50);
    click("GettingStarted.tryAgain");
    d.check(QStringLiteral("qp17: Try Again starts over"), installer.resets == 1 && !wizard->property("installChosen").toBool());
    installer.set(QStringLiteral("verified"));
    d.spin(50);
    d.check(QStringLiteral("qp17: a verified install offers Continue"),
            qp17Item(wizard, QStringLiteral("GettingStarted.continue"))->property("visible").toBool());
    click("GettingStarted.continue");

    shown = shownNow();
    const QVariantMap done = qp17Stage(words, "done");
    d.check(QStringLiteral("qp17: Done shows the core's paragraphs and rows, resolved"),
            wizard->property("current").toInt() == 2 && qp17AllShown(done.value(QStringLiteral("paragraphs")).toList(), shown)
                && qp17AllShown(done.value(QStringLiteral("rows")).toList(), shown) && qp17Resolved(shown));
    const QString joined = shown.join(QLatin1Char('\n'));
    d.check(QStringLiteral("qp17: Done names no host device and this app's settings page"),
            !joined.contains(QLatin1String("computer")) && joined.contains(QLatin1String("under Outputs")));
    d.check(QStringLiteral("qp17: the last button has the core's word"), shown.contains(buttons.value(QStringLiteral("finish")).toString()));
    d.grab(d.mainWindow(), QStringLiteral("window-wizard-done.png"));
    click("GettingStarted.continue");
    d.spin(200);
    d.check(QStringLiteral("qp17: finishing hands the console back for good"),
            !ob->wizardShown() && ob->setupDone() && !wizardNow() && qobject_cast<QQuickItem *>(pane)->isVisible());
    d.check(QStringLiteral("qp17: the wizard let go of the installer"), installer.watching == 0);

    // Skip, from the first step
    ob->requestSetup();
    d.spin(300);
    wizard = wizardNow();
    if (QObject *skip = wizard ? qp17Item(wizard, QStringLiteral("GettingStarted.skip")) : nullptr) QMetaObject::invokeMethod(skip, "click");
    d.spin(200);
    d.check(QStringLiteral("qp17: Skip Setup leaves at once, for good"), !ob->wizardShown() && ob->setupDone());
    ob->setInstaller(nullptr);

    // A first start: the flag unset shows the wizard (as a new controller sees it)
    QSettings().remove(QStringLiteral("onboarding/setupDone"));
    d.check(QStringLiteral("qp17: without the flag, setup is not done"), !ob->setupDone());
    ob->finishSetup();

    // ---- About's licence block ----------------------------------------------
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    if (!w) {
        d.check(QStringLiteral("qp17: the Settings window opens"), false);
        return;
    }
    settings->open(QStringLiteral("about"));
    d.spin(400);
    const QVariantMap licence = ob->licence();
    QObject *block = qp17Item(w->contentItem(), QStringLiteral("About.licence"));
    QObject *statement = qp17Item(w->contentItem(), QStringLiteral("About.licenceStatement"));
    int terms = 0;
    for (const QVariant &t : licence.value(QStringLiteral("terms")).toList())
        if (qp17Item(w->contentItem(), QStringLiteral("About.licenceTerm.") + t.toMap().value(QStringLiteral("id")).toString())) ++terms;
    d.check(QStringLiteral("qp17: About shows the core's licence statement"),
            block && block->property("visible").toBool() && statement
                && statement->property("text").toString() == licence.value(QStringLiteral("statement")).toString());
    d.check(QStringLiteral("qp17: and each additional term"),
            terms == licence.value(QStringLiteral("terms")).toList().size() && terms > 0);
    QObject *gplLink = qp17Item(w->contentItem(), QStringLiteral("About.licenceGpl"));
    const bool gplStaged = [&]() {
        for (const QVariant &n : Registrar::instance<AboutController>()->notices())
            if (n.toMap().value(QStringLiteral("path")).toString().endsWith(QLatin1String("GPL-3.0.txt"))) return true;
        return false;
    }();
    d.check(QStringLiteral("qp17: the GPL's web page only when its text is not staged"),
            gplLink && gplLink->property("visible").toBool() == !gplStaged);
    if (auto *section = qobject_cast<QQuickItem *>(block)) {
        // Scroll the licence into view for the screenshot
        for (QQuickItem *p = section->parentItem(); p; p = p->parentItem())
            if (p->inherits("QQuickFlickable")) {
                p->setProperty("contentY", section->mapToItem(p->property("contentItem").value<QQuickItem *>(), 0, 0).y());
                break;
            }
    }
    d.spin(100);
    d.grab(w, QStringLiteral("settings-about-licence.png"));

    // ---- Advanced: the developer rows behind Show Debug Information ---------
    const bool debugBefore = settings->showDebugInfo();
    settings->setShowDebugInfo(false);
    settings->open(QStringLiteral("advanced"));
    d.spin(400);
    QObject *dev = qp17Item(w->contentItem(), QStringLiteral("Advanced.onboarding"));
    d.check(QStringLiteral("qp17: the onboarding rows hide without debug information"), dev && !dev->property("visible").toBool());
    settings->setShowDebugInfo(true);
    d.spin(100);
    d.check(QStringLiteral("qp17: and show with it"), dev && dev->property("visible").toBool());
    if (QObject *reset = qp17Item(w->contentItem(), QStringLiteral("Advanced.resetOnboarding"))) QMetaObject::invokeMethod(reset, "click");
    d.spin(100);
    d.check(QStringLiteral("qp17: Reset Onboarding forgets setup and the notes"),
            !ob->setupDone() && ob->whatsNewShown().isEmpty() && !ob->wizardShown());
    d.grab(w, QStringLiteral("settings-advanced-onboarding.png"));
    ob->finishSetup();
    ob->markWhatsNewRead();
    settings->setShowDebugInfo(debugBefore);
    w->setVisible(false);
    d.spin(100);
    d.check(QStringLiteral("qp17: left as found"), ob->setupDone() && ob->whatsNewShown() == ob->bundledFirmware());
}

} // namespace

const SmokeDriver::Section qp17Onboarding("qp17-onboarding", 160, &onboardingSection);

#include "SmokeOnboarding.moc"
