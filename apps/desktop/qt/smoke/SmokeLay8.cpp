// -- WP LAY8 -- Modern's AI button and its popover over a fake controller: the
// states (off, waiting, connected; paused, asking, a failed start), the switch's
// question, Disconnect, the latest changes, the ways to the settings and the
// activity, and the popover at 1000x660. Pictures go to gallery/lay8-*. The real
// server's part is in mcp4-ai.
#include "SmokeDriver.h"

#include "AiController.h"
#include "LayoutController.h"
#include "Registrar.h"
#include "Shell.h"

#include <QColor>
#include <QCoreApplication>
#include <QDir>
#include <QJSValue>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSet>
#include <QTest>
#include <cstdio>
#include <memory>

namespace {

// AiController's properties as the button and the popover read them
const char *const kFake = R"(
import QtQml
QtObject {
    property string state: "off"
    property string statusLine: "AI agents are off."
    property bool enabled: false
    property bool running: false
    property bool busy: false
    property bool paused: false
    property string lastError: ""
    property string confirmMode: "ask_risky"
    property var addresses: []
    property var clients: []
    property var agents: []
    property var activity: []
    property var calls: []
    function did(call) { calls = calls.concat([call]) }
    function refresh() { did("refresh") }
    function setEnabled(on) { did("enabled " + on); enabled = on; if (on) waiting(); else { state = "off"; running = false; addresses = []; agents = [] } }
    function waiting() { state = "on"; statusLine = "Waiting for AI agents."; running = true; addresses = ["127.0.0.1:47860", "[::1]:47860"] }
    function connect() {
        clients = [{ id: 7, name: "<b>Claude Code</b>" }, { id: 9, name: "Cursor" }]
        agents = [{ id: 7, name: "<b>Claude Code</b>", since: "14:02" }, { id: 9, name: "Cursor", since: "14:05" }]
        state = "active"; statusLine = "An AI agent is active."
    }
    function many() {
        const rows = []
        for (let i = 0; i < 12; ++i) rows.push({ id: 20 + i, name: "Agent " + i, since: "14:10" })
        agents = rows
    }
    function row(key, undo, note, write, outcome) {
        return { key: key, time: "14:06", client: "AI · Cursor", title: "Set a gain", detail: "Out 1 output gain -10 dB to -15 dB", device: "Demo",
                 write: write, outcome: outcome || "ok", outcomeText: write ? "Done" : "Read", undo: undo, undoNote: note }
    }
    function work() {
        activity = [row("1:9", "", "", true, "held"), row("1:8", "", "", true, "error"), row("1:7", "", "", false), row("1:6", "can", "", true), row("1:5", "changed", "Changed since: gain", true), row("1:4", "undone", "", true),
                    row("1:3", "can", "", true), row("1:2", "can", "", true)]
    }
    function disconnectAgent(id) { did("disconnect " + id); agents = agents.filter(a => a.id !== id); if (!agents.length) waiting() }
    function setPaused(on) { did("paused " + on); paused = on; state = on ? "paused" : "on" }
    function dismissError() { did("retry") }
    function undo(key) { did("undo " + key) }
    function openSettings() { did("settings") }
    function openPanel() { did("panel") }
    function openLimits() { did("limits") }
})";

struct Lay8 {
    SmokeDriver &d;
    QQuickWindow *main;
    QObject *fake;
    QQuickItem *button = nullptr;
    QObject *popover = nullptr;

    void check(const char *what, bool ok) const { d.check(QStringLiteral("lay8: ") + QString::fromUtf8(what), ok); }
    QQuickItem *item(const char *name) const { return d.item(QString::fromLatin1(name), main->contentItem()->parentItem()); }
    bool shows(const char *name) const {
        QQuickItem *it = item(name);
        return it && it->isVisible();
    }
    QString text(const char *name) const {
        QQuickItem *it = item(name);
        return it && it->isVisible() ? it->property("text").toString() : QString();
    }
    void clickOn(QQuickItem *it) const {
        if (!it) return;
        const QPointF at = it->mapToScene(QPointF(it->width() / 2, it->height() / 2));
        QTest::mouseClick(main, Qt::LeftButton, Qt::NoModifier, at.toPoint());
        d.spin(150);
    }
    bool click(const char *name) const {
        QQuickItem *it = item(name);
        if (!it || !it->isVisible() || !it->isEnabled()) return false;
        clickOn(it);
        return true;
    }
    bool opened() const { return popover->property("opened").toBool(); }
    bool open() const {
        if (!opened()) clickOn(button);
        return d.waitFor(2000, [&] { return opened(); });
    }
    QString face() const { return button->property("face").toString(); }
    void set(const char *property, const QVariant &value) const {
        fake->setProperty(property, value);
        d.spin(50);
    }
    void run(const char *method) const {
        QMetaObject::invokeMethod(fake, method);
        d.spin(100);
    }
    QStringList calls() const {
        const QVariant v = fake->property("calls");
        return v.canConvert<QStringList>() ? v.toStringList() : v.value<QJSValue>().toVariant().toStringList();
    }
    QString last() const { return calls().isEmpty() ? QString() : calls().last(); }
    // Inside the window, whole
    bool fits() const {
        QQuickItem *body = popover->property("contentItem").value<QQuickItem *>();
        if (!body) return false;
        const QRectF at = body->mapRectToScene(QRectF(0, 0, body->width(), body->height()));
        return at.width() > 200 && at.height() > 80 && QRectF(0, 0, main->width(), main->height()).contains(at);
    }
    void picture(const char *name) const {
        auto *layout = Registrar::instance<LayoutController>();
        const QString theme = layout->theme();
        for (const char *scheme : { "dark", "light" }) {
            layout->setTheme(QString::fromLatin1(scheme));
            d.spin(250);
            d.grab(main, QStringLiteral("gallery/lay8-%1-%2.png").arg(QLatin1String(name), QLatin1String(scheme)));
        }
        layout->setTheme(theme);
        d.spin(100);
    }
};

void states(Lay8 &t) {
    QQuickItem *mark = t.d.item(QStringLiteral("AiButton.mark"));
    const auto ring = [&] { return mark && mark->property("ring").toBool(); };
    QQuickItem *background = t.button->property("background").value<QQuickItem *>();

    QQuickItem *search = t.d.item(QStringLiteral("TopBar.search")), *theme = t.d.item(QStringLiteral("TopBar.theme"));
    t.check("the button is labelled AI, between the search and the theme switch",
            t.button->property("text") == QLatin1String("AI") && search && theme && search->x() < t.button->x() && t.button->x() < theme->x());

    // ---- Off
    t.check("off: an empty ring", t.face() == QLatin1String("off") && ring());
    const QColor plain = background ? background->property("color").value<QColor>() : QColor();
    t.check("a click opens the popover, which reads the state anew", t.open() && t.last() == QLatin1String("refresh"));
    t.check("off: it says so, the switch off, nothing to pause", t.text("AiPopover.status") == QLatin1String("AI agents are off.")
            && t.shows("AiPopover.off") && !t.item("AiPopover.allow")->property("checked").toBool() && !t.shows("AiPopover.addresses")
            && !t.shows("AiPopover.pause") && !t.shows("AiPopover.connect") && t.shows("AiPopover.settings"));
    t.check("it hangs inside the window, at its button", t.fits());
    t.picture("off");
    t.check("the switch has the focus", t.item("AiPopover.allow")->hasActiveFocus());
    QTest::keyClick(t.main, Qt::Key_Escape);
    t.check("Escape closes it", t.d.waitFor(1000, [&] { return !t.opened(); }));
    t.open();
    t.clickOn(t.button);
    t.check("the button closes it again", t.d.waitFor(1000, [&] { return !t.opened(); }));

    // ---- On asks first
    t.open();
    QObject *question = t.popover->property("enableDialog").value<QObject *>();
    const auto asked = [&] { return t.d.waitFor(2000, [&] { return question && question->property("opened").toBool(); }); };
    t.click("AiPopover.allow");
    t.check("turning them on asks first", asked() && !t.fake->property("enabled").toBool());
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    t.d.spin(300);
    t.check("Cancel leaves them off, and the switch", !t.calls().contains(QStringLiteral("enabled true")) && t.opened()
            && !t.item("AiPopover.allow")->property("checked").toBool());
    t.click("AiPopover.allow");
    asked();
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    t.check("Allow turns them on", t.d.waitFor(1000, [&] { return t.calls().contains(QStringLiteral("enabled true")); }));
    t.d.spin(300);

    // ---- Waiting
    t.check("waiting: a ring of another colour", t.face() == QLatin1String("waiting") && ring());
    t.check("waiting: the addresses, and when agents ask", t.text("AiPopover.status") == QLatin1String("Waiting for AI agents.")
            && t.text("AiPopover.addresses").contains(QLatin1String("127.0.0.1:47860")) && t.text("AiPopover.asks").contains(QLatin1String("risky"))
            && !t.shows("AiPopover.off") && t.item("AiPopover.allow")->property("checked").toBool());
    t.set("confirmMode", QStringLiteral("never_ask"));
    const QString never = t.text("AiPopover.asks");
    t.set("confirmMode", QStringLiteral("ask_everything"));
    t.check("each setting has its words", never.contains(QLatin1String("without")) && t.text("AiPopover.asks").contains(QLatin1String("every")));
    t.set("confirmMode", QStringLiteral("ask_risky"));
    t.check("the mark stops pulsing", t.d.waitFor(7000, [&] { return mark->opacity() == 1; })
            && [&] { t.d.spin(400); return mark->opacity() == 1; }());
    t.picture("waiting");
    t.check("with no client it leads to connecting one, in the settings", t.click("AiPopover.connect") && t.last() == QLatin1String("settings")
            && t.d.waitFor(1000, [&] { return !t.opened(); }));

    // ---- Connected
    t.run("connect");
    t.run("work");
    const QColor lit = background ? background->property("color").value<QColor>() : QColor();
    t.check("connected: a dot on a highlighted button", t.face() == QLatin1String("connected") && !ring() && lit.isValid() && lit != plain);
    t.open();
    QQuickItem *name = t.item("AiPopover.agent.0.name");
    t.check("connected: who, as the name is written", name && name->property("text") == QLatin1String("<b>Claude Code</b>")
            && name->property("textFormat").toInt() == 0 && t.text("AiPopover.agent.1.name") == QLatin1String("Cursor") && !t.shows("AiPopover.connect"));
    t.check("the four latest changes made, no reads and no questions", t.shows("AiPopover.change.3") && !t.item("AiPopover.change.4")
            && t.text("AiPopover.change.0.title").contains(QLatin1String("-15 dB")));
    t.check("Undo where there is something to undo, and why not elsewhere", t.shows("AiPopover.change.0.undo") && !t.shows("AiPopover.change.1.undo")
            && t.text("AiPopover.change.1.who") == QLatin1String("Changed since: gain") && !t.shows("AiPopover.change.2.undo"));
    t.check("it fits", t.fits());
    t.picture("connected");
    t.check("Undo undoes that change", t.click("AiPopover.change.0.undo") && t.last() == QLatin1String("undo 1:6"));

    // ---- Disconnect
    t.check("Disconnect of the first is of that one", t.click("AiPopover.agent.0.disconnect") && t.last() == QLatin1String("disconnect 7")
            && t.text("AiPopover.agent.0.name") == QLatin1String("Cursor") && !t.item("AiPopover.agent.1") && t.face() == QLatin1String("connected"));
    t.check("and of the last leaves them waiting, still on", t.click("AiPopover.agent.0.disconnect") && t.last() == QLatin1String("disconnect 9")
            && t.face() == QLatin1String("waiting") && t.fake->property("enabled").toBool() && t.opened());

    // ---- Paused, asking, a failed start
    t.check("Pause", t.click("AiPopover.pause") && t.last() == QLatin1String("paused true") && t.face() == QLatin1String("paused"));
    t.picture("paused");
    t.check("and Resume", t.click("AiPopover.pause") && t.last() == QLatin1String("paused false"));
    t.set("state", QStringLiteral("asking"));
    t.check("a question closes the popover; the button says so", t.face() == QLatin1String("attention")
            && t.d.waitFor(1000, [&] { return !t.opened(); }));
    t.set("running", false);
    t.set("state", QStringLiteral("error"));
    t.set("lastError", QStringLiteral("Port 47860 is in use."));
    t.open();
    t.check("a failed start: why, and Try Again", t.face() == QLatin1String("attention") && t.text("AiPopover.error") == QLatin1String("Port 47860 is in use.")
            && t.click("AiPopover.retry") && t.last() == QLatin1String("retry"));
    t.picture("error");
    t.set("lastError", QString());
    t.run("waiting");

    // ---- The ways on
    t.check("Activity", t.click("AiPopover.activity") && t.last() == QLatin1String("panel") && t.d.waitFor(1000, [&] { return !t.opened(); }));
    t.open();
    t.check("Settings", t.click("AiPopover.settings") && t.last() == QLatin1String("settings") && t.d.waitFor(1000, [&] { return !t.opened(); }));
    t.open();
    t.check("the switch turns them off without asking", t.click("AiPopover.allow") && t.last() == QLatin1String("enabled false")
            && t.face() == QLatin1String("off") && !question->property("opened").toBool());
    QMetaObject::invokeMethod(t.popover, "close");
    t.d.spin(200);
}

// The smallest main window, and more agents than it has room for
void narrow(Lay8 &t) {
    const QSize was = t.main->size();
    t.main->resize(1000, 660);
    t.d.spin(300);
    t.run("waiting");
    t.set("enabled", true);
    t.run("connect");
    t.run("many");
    t.run("work");
    const QRectF at = t.button->mapRectToScene(QRectF(0, 0, t.button->width(), t.button->height()));
    t.check("at 1000x660 the button shows, with its word", t.button->isVisible() && at.right() <= t.main->width()
            && t.button->width() >= t.button->implicitWidth());
    t.open();
    QObject *body = t.popover->property("contentItem").value<QObject *>();
    t.check("the popover stays inside, and scrolls", t.fits() && body && body->property("contentHeight").toReal() > body->property("height").toReal());
    t.picture("1000");
    QMetaObject::invokeMethod(t.popover, "close");
    t.main->resize(was);
    t.d.spin(300);
}

void lay8Section(SmokeDriver &d) {
    auto *layout = Registrar::instance<LayoutController>();
    auto *ai = Registrar::instance<AiController>();
    QQuickWindow *main = d.mainWindow();
    d.check("lay8: the controllers exist", layout && ai && main);
    if (!layout || !ai || !main) return;
    QDir(d.outDir()).mkpath(QStringLiteral("gallery"));

    // The confirmation sheet's labels: each of dspi-mcp's kinds, none twice
    QSet<QString> labels;
    const QStringList kinds{ QStringLiteral("louder"), QStringLiteral("unmute"), QStringLiteral("enable"), QStringLiteral("new_route"),
                             QStringLiteral("boost"), QStringLiteral("delay_step"), QStringLiteral("tweeter_crossing"), QStringLiteral("change"),
                             QStringLiteral("test_signal"), QStringLiteral("recall"), QStringLiteral("flash_write"),
                             QStringLiteral("installation_save"), QStringLiteral("source"), QStringLiteral("system") };
    for (const QString &kind : kinds) labels.insert(AiController::riskLabel(kind));
    labels.remove(QString());
    d.check("lay8: each kind of risk has a label of its own", labels.size() == kinds.size());
    d.check("lay8: a kind this app does not know has none", AiController::riskLabel(QStringLiteral("newer")).isEmpty());

    const QString before = layout->layout();
    layout->setLayout(QStringLiteral("modern"));
    d.waitFor(3000, [&] { return d.item(QStringLiteral("TopBar.ai")) != nullptr; });
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    d.waitIdle();
    d.spin(300);

    QQmlComponent component(d.app().engine);
    component.setData(QByteArray(kFake), QUrl(QStringLiteral("lay8.qml")));
    std::unique_ptr<QObject> fake(component.create());
    if (!fake) std::printf("smoke: lay8: %s\n", qPrintable(component.errorString()));
    Lay8 t{ d, main, fake.get() };
    t.button = d.item(QStringLiteral("TopBar.ai"));
    t.popover = t.button ? t.button->property("popover").value<QObject *>() : nullptr;
    d.check("lay8: the AI button and its popover are there", fake && t.button && t.popover);
    if (fake && t.button && t.popover) {
        d.check("lay8: they are AiController's", t.button->property("ai").value<QObject *>() == ai
                && t.popover->property("ai").value<QObject *>() == ai);
        t.button->setProperty("ai", QVariant::fromValue(fake.get()));
        d.spin(50);
        states(t);
        narrow(t);
        t.button->setProperty("ai", QVariant::fromValue<QObject *>(ai));
        d.spin(50);
    }
    layout->setLayout(before);
    d.waitIdle();
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    d.spin(300);
}

const SmokeDriver::Section lay8("lay8-ai", 983, &lay8Section);

} // namespace
