// WP QD9: the Devices list and the PIN prompt, through the real QML, on fake hubs.
// One list holds the demo and a hub's device; a found hub is held to the id it
// announced (wrong_hub); the prompt shows by itself, doesn't block, takes a wrong PIN
// and a right one, and Cancel leaves; every LinkPhase shows CG10's sentence, both
// PIN sources and reconnecting's button included; choosing the demo leaves the hub.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "DemoDevice.h"
#include "HubController.h"
#include "LinkController.h"
#include "LinkFakeHub.h"

#include <QCoreApplication>
#include <QJSValue>
#include <QJsonDocument>
#include <QKeyEvent>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTimer>
#include <QVariantMap>

using namespace linksmoke;

namespace {

// CG10's sentence, straight from the core: what the list must show
QString cg10Text(const char *phase, const char *pinSource = "") {
    char buf[2048];
    const uint32_t needed = dspi_link_phase_text(phase, pinSource, buf, sizeof(buf));
    if (needed == 0 || needed > sizeof(buf)) return QString();
    return QJsonDocument::fromJson(QByteArray(buf, int(needed) - 1)).object().value(QStringLiteral("text")).toString();
}

QQuickItem *contentOf(QObject *popup) {
    return popup ? popup->property("contentItem").value<QQuickItem *>() : nullptr;
}

QQuickItem *in(QObject *popup, const QString &name) {
    QQuickItem *content = contentOf(popup);
    return content ? findItem(content, name) : nullptr;
}

bool click(QQuickItem *item) {
    return item && item->isVisible() && item->isEnabled() && QMetaObject::invokeMethod(item, "clicked");
}

}

void SmokeDriver::testDevices() {
    section("QD9: Devices list and PIN prompt on the fake hub");
    QQmlContext *context = m_.engine->rootContext();
    auto *link = qobject_cast<LinkController *>(context->contextProperty(QStringLiteral("linkSession")).value<QObject *>());
    auto *hubs = qobject_cast<HubController *>(context->contextProperty(QStringLiteral("hub")).value<QObject *>());
    QObject *popup = root()->findChild<QObject *>(QStringLiteral("devicesPopup"));
    QObject *dialog = root()->findChild<QObject *>(QStringLiteral("pairingDialog"));
    check("QD9: the list and the prompt are loaded", link && hubs && popup && dialog);
    if (!link || !hubs || !popup || !dialog) return;
    FakeHubProcess hub;
    const QString pin = QStringLiteral("482915");
    const bool hubUp = hub.start({ "--auth", QStringLiteral("window:") + pin });
    check("QD9: the fake hub runs (DSPI_SMOKE_FAKEHUB)", hubUp);
    if (!hubUp) return;
    auto *b = m_.bridge;
    auto *main = qobject_cast<QQuickWindow *>(root());
    const auto grab = [&](const char *name) {
        if (main) main->grabWindow().save(m_out + QStringLiteral("/window-devices-%1.png").arg(QLatin1String(name)));
    };
    const auto settled = [&] { return waitFor(30000, [link] { return !link->busy(); }); };
    const auto phaseIs = [link](const char *phase) { return link->phase() == QLatin1String(phase); };
    const auto opened = [](QObject *p) { return p->property("opened").toBool(); };
    // The list's status line says CG10's sentence for this phase, and nothing else
    const auto shows = [&](const char *phase, const char *pinSource = "") {
        QQuickItem *line = in(popup, QStringLiteral("devicesPhaseText"));
        const QString want = cg10Text(phase, pinSource);
        return phaseIs(phase) && !want.isEmpty() && line && line->isVisible() && link->phaseText() == want
               && line->property("text").toString().endsWith(want);
    };
    // The unsaved-changes question a switch may ask first: discard
    const auto answerUnsaved = [&] {
        for (QObject *o : root()->findChildren<QObject *>()) {
            if (o->property("title").toString() == QLatin1String("Unsaved Changes") && opened(o)) {
                QMetaObject::invokeMethod(o, "choose", Q_ARG(QVariant, QStringLiteral("discard")));
                return true;
            }
        }
        return false;
    };
    const auto hubRow = [&](const QString &serial) { return in(popup, QStringLiteral("devicesHub_") + serial); };
    const bool demoBefore = b->connected() && b->selectedSerial() == QLatin1String(DemoDevice::kSerial);
    const bool watchedBefore = hubs->watching();

    // Opened from the Device menu's entry
    check("QD9: idle before", phaseIs("idle"));
    QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("devices")));
    check("QD9: Devices… opens the list", waitFor(2000, [&] { return opened(popup); }));
    check("QD9: and looks for hubs while it shows", hubs->watching() && (hubs->browsing() || !hubs->browseError().isEmpty()));
    QQuickItem *demoRow = in(popup, QStringLiteral("devicesDemo"));
    check("QD9: the demo is in the list, marked open", demoRow && demoRow->isVisible() && demoRow->property("current").toBool() == demoBefore);
    check("QD9: under --demo the demo is not listed again as USB",
          !in(popup, QStringLiteral("devicesUsb_") + QLatin1String(DemoDevice::kSerial)));
    check("QD9: idle in CG10's words", shows("idle"));
    grab("idle");

    // A hub found on the network whose hello names another id (owner Q7): held to
    // the id it announced, so no session and no token
    QVariantMap spoofed {
        { QStringLiteral("id"), QStringLiteral("00000000-0000-4000-8000-0000000000d9") },
        { QStringLiteral("name"), QStringLiteral("Not That Hub") },
        { QStringLiteral("url"), hub.url },
    };
    QMetaObject::invokeMethod(popup, "connectFound", Q_ARG(QVariant, QVariant(spoofed)));
    QQuickItem *error = in(popup, QStringLiteral("devicesError"));
    check("QD9: a found hub with another id is refused", settled() && link->errorCode() == QLatin1String("wrong_hub"));
    check("QD9: the list says so", error && error->isVisible() && error->property("text").toString() == link->errorText());
    check("QD9: and no session, no prompt", phaseIs("idle") && !opened(dialog));

    // By address, from the list's own field: the hub asks, the prompt shows by itself
    QQuickItem *address = in(popup, QStringLiteral("devicesAddress"));
    if (address) address->setProperty("text", hub.address);
    check("QD9: Connect by address", click(in(popup, QStringLiteral("devicesConnect"))));
    check("QD9: the hub asks for its PIN", settled() && waitFor(3000, [&] { return phaseIs("needs_pairing"); }));
    check("QD9: the prompt shows by itself", waitFor(2000, [&] { return opened(dialog); }));
    QQuickItem *prompt = in(dialog, QStringLiteral("pairingText"));
    const QString windowText = cg10Text("needs_pairing", "window");
    check("QD9: needs_pairing (window) in CG10's words, in the prompt and the list",
          shows("needs_pairing", "window") && prompt && prompt->property("text").toString() == windowText);
    // It doesn't block: the event loop runs while it shows
    int ticks = 0;
    QTimer ticker;
    QObject::connect(&ticker, &QTimer::timeout, [&] { ticks++; });
    ticker.start(20);
    spin(300);
    ticker.stop();
    check("QD9: the prompt does not block", ticks >= 5 && opened(dialog));
    grab("pin");

    // A wrong PIN: the prompt stays, says why, and is empty again
    QQuickItem *pinField = in(dialog, QStringLiteral("pairingPin"));
    QQuickItem *pairButton = in(dialog, QStringLiteral("pairingPair"));
    QQuickItem *pinError = in(dialog, QStringLiteral("pairingError"));
    check("QD9: Pair is off until a PIN is typed", pairButton && !pairButton->isEnabled());
    // -- WP QD13b -- (review c4q N1) Six digits or nothing: a typo costs no try
    if (pinField && pairButton) {
        const auto typeKeys = [&](const QString &keys) {
            for (const QChar c : keys) {
                const int key = c == QLatin1Char(' ') ? int(Qt::Key_Space) : int(c.toUpper().unicode());
                QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier, QString(c));
                QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier, QString(c));
                QCoreApplication::sendEvent(pinField, &press);
                QCoreApplication::sendEvent(pinField, &release);
            }
        };
        pinField->forceActiveFocus();
        typeKeys(QStringLiteral("a1 2-345x67"));
        check("QD13b: the PIN field takes six digits and nothing else", pinField->property("text").toString() == QLatin1String("123456")
              && pairButton->isEnabled());
        printf("smoke: QD13b typed into the PIN field: '%s'\n", qPrintable(pinField->property("text").toString()));
        const QString before = link->errorCode();
        for (const char *bad : { "12345", "12345a", "12 456", "" }) {
            pinField->setProperty("text", QString::fromLatin1(bad));
            QMetaObject::invokeMethod(dialog, "submit");
            check("QD13b: Pair is off, and a submit sends nothing, for a PIN that is not six digits",
                  !pairButton->isEnabled() && !link->busy() && link->errorCode() == before && phaseIs("needs_pairing"));
        }
    }
    if (pinField) pinField->setProperty("text", QStringLiteral("000000"));
    check("QD9: Pair sends it", click(pairButton));
    check("QD9: wrong PIN", settled() && link->errorCode() == QLatin1String("wrong_pin"));
    check("QD9: the prompt stays and says so", waitFor(1000, [&] {
        return opened(dialog) && pinError && pinError->isVisible() && !pinError->property("text").toString().isEmpty();
    }));
    check("QD9: and is cleared for another try", pinField && pinField->property("text").toString().isEmpty());
    grab("pin-wrong");

    // The right one: past pairing, the hub's device in the list
    if (pinField) pinField->setProperty("text", pin);
    check("QD9: Pair again", click(pairButton));
    check("QD9: paired", settled() && link->errorCode().isEmpty());
    check("QD9: the prompt goes", waitFor(3000, [&] { return !opened(dialog); }));
    check("QD9: choose_device in CG10's words", waitFor(3000, [&] { return shows("choose_device") && link->devices().size() == 1; }));
    const QString serial = link->devices().value(0).toMap().value(QStringLiteral("serial")).toString();
    QQuickItem *row = nullptr;
    check("QD9: the hub's device is in the same list", waitFor(2000, [&] { return (row = hubRow(serial)) && row->isVisible(); }));
    grab("choose");

    // Chosen from the list: bound, shown and marked
    check("QD9: choose the hub's device", click(row));
    waitFor(500, answerUnsaved);
    check("QD9: bound in CG10's words", settled() && waitFor(5000, [&] { return shows("bound"); }));
    check("QD9: the bridge shows it", b->connected() && b->selectedSerial() == serial);
    check("QD9: marked open, and the demo is not", waitFor(1000, [&] {
        return (row = hubRow(serial)) && row->property("current").toBool() && demoRow && !demoRow->property("current").toBool();
    }));
    grab("bound");

    // no_device: the hub's board goes
    check("QD9: the hub takes board off", hub.fault("board off"));
    check("QD9: no_device in CG10's words", waitFor(5000, [&] { return shows("no_device"); }));
    check("QD9: the hub takes board on", hub.fault("board on"));
    check("QD9: the device is back", waitFor(10000, [&] { return phaseIs("bound") || phaseIs("choose_device"); }));
    if (phaseIs("choose_device")) {
        check("QD9: choose it again", waitFor(2000, [&] { return (row = hubRow(serial)) != nullptr; }) && click(row));
        waitFor(500, answerUnsaved);
    }
    check("QD9: bound again", settled() && waitFor(5000, [&] { return phaseIs("bound"); }));

    // reconnecting, with CG10's button and a retry
    check("QD9: the hub takes refuse 503", hub.fault("refuse 503"));
    check("QD9: the hub takes drop", hub.fault("drop"));
    QQuickItem *action = in(popup, QStringLiteral("devicesPhaseAction"));
    QQuickItem *retry = in(popup, QStringLiteral("devicesRetry"));
    check("QD9: reconnecting in CG10's words", waitFor(3000, [&] { return shows("reconnecting"); }));
    check("QD9: with CG10's button", action && action->isVisible() && link->actionKey() == QLatin1String("reconnecting.disconnect")
                                         && action->property("text").toString() == link->actionText());
    check("QD9: and Retry Now", retry && retry->isVisible());
    check("QD9: the hub's device can't be chosen meanwhile", (row = hubRow(serial)) && !row->isEnabled());
    grab("reconnecting");
    check("QD9: the hub takes refuse ok", hub.fault("refuse ok"));
    check("QD9: Retry Now", click(retry));
    check("QD9: bound after the retry", waitFor(15000, [&] { return phaseIs("bound"); }) && !action->isVisible());

    // closed: the hub ends the session for good
    check("QD9: the hub takes close 4000", hub.fault("close 4000"));
    check("QD9: closed in CG10's words", waitFor(5000, [&] { return shows("closed"); }));
    check("QD9: no hub devices listed once closed", !hubRow(serial));
    grab("closed");

    // Again by address: the stored token, no prompt this time
    if (address) address->setProperty("text", hub.address);
    check("QD9: Connect again", click(in(popup, QStringLiteral("devicesConnect"))));
    check("QD9: no PIN asked the second time", settled() && waitFor(3000, [&] { return phaseIs("choose_device"); }) && !opened(dialog));

    // refused: the hub turns the reconnect away (4xx)
    check("QD9: the hub takes refuse 403", hub.fault("refuse 403"));
    check("QD9: the hub takes drop again", hub.fault("drop"));
    check("QD9: lost", waitFor(3000, [&] { return phaseIs("reconnecting") || phaseIs("refused"); }));
    if (phaseIs("reconnecting")) link->retryNow();
    check("QD9: refused in CG10's words", waitFor(15000, [&] { return shows("refused"); }));
    grab("refused");
    check("QD9: the hub takes refuse ok again", hub.fault("refuse ok"));

    // Choosing the demo leaves the hub, which owns the connection while it is up
    if (address) address->setProperty("text", hub.address);
    check("QD9: Connect a third time", click(in(popup, QStringLiteral("devicesConnect"))));
    check("QD9: on the hub", settled() && waitFor(3000, [&] { return phaseIs("choose_device"); }));
    check("QD9: choose the demo", click(demoRow));
    waitFor(500, answerUnsaved);
    check("QD9: the demo is open and the hub left", waitFor(5000, [&] {
        return phaseIs("idle") && b->connected() && b->selectedSerial() == QLatin1String(DemoDevice::kSerial)
               && demoRow->property("current").toBool();
    }));
    settled();

    // -- WP QD37 -- The status tooltip during a hub session (review c4o N1), and one
    // unsaved-changes ask on a USB pick that leaves the hub (c4q N2). Under --demo the
    // demo's serial is in the bridge's USB list, so it stands in for a USB device.
    {
        QQuickItem *cpuStatus = main ? findItem(main->contentItem(), QStringLiteral("cpuStatus")) : nullptr;
        const auto tip = [&] { return cpuStatus ? cpuStatus->property("statusTip").toString() : QString(); };
        check("QD37: the status tooltip offers a reconnect off the hub", tip().contains(QLatin1String("Right-click to reconnect")));
        int hubSessionNotes = 0;
        const auto noted = QObject::connect(b, &DSPiBridge::hubSessionChanged, [&] { hubSessionNotes++; });
        if (address) address->setProperty("text", hub.address);
        check("QD37: Connect a fourth time", click(in(popup, QStringLiteral("devicesConnect"))));
        check("QD37: on the hub", settled() && waitFor(3000, [&] { return phaseIs("choose_device"); }));
        check("QD37: hubSession is a property, and says so", b->property("hubSession").toBool() && hubSessionNotes > 0);
        check("QD37: choose the hub's device", waitFor(2000, [&] { return (row = hubRow(serial)) != nullptr; }) && click(row));
        waitFor(500, answerUnsaved);
        check("QD37: bound", settled() && waitFor(5000, [&] { return phaseIs("bound") && b->connected(); }));
        printf("smoke: QD37 status tooltip on the hub: '%s'\n", qPrintable(tip().replace(QLatin1Char('\n'), QLatin1String(" | "))));
        check("QD37: the tooltip says it is a hub's device and offers no right-click",
              tip().contains(QLatin1String("through a hub")) && tip().contains(QStringLiteral("Device > Devices"))
                  && !tip().contains(QLatin1String("Right-click")));
        b->setFilter(0, 0, 1, b->filterFreq(0, 0) + 17.0f, 1.0f, 0.9f);
        check("QD37: unsaved changes on the hub's device", waitFor(2000, [&] { return b->presetDirty(); }));
        // Here the bridge drops the hub's device as the session closes, so a second
        // ask would find nothing unsaved; on a slower close it would not. So count the
        // calls into both of main.qml's asking paths, guard and switchTo, as well as
        // the questions shown.
        QJSValue counter = m_.engine->evaluate(QStringLiteral(
            "(function (pop) { var n = { calls: 0, guard: pop.guard, switchTo: pop.switchTo };"
            " pop.guard = function (a) { n.calls++; n.guard(a) };"
            " pop.switchTo = function (s) { n.calls++; n.switchTo(s) }; return n })"));
        QJSValue calls = counter.call({ m_.engine->newQObject(popup) });
        int asks = 0;
        QMetaObject::invokeMethod(popup, "chooseUsb", Q_ARG(QVariant, QString::fromLatin1(DemoDevice::kSerial)));
        if (waitFor(1000, answerUnsaved)) asks++;
        while (waitFor(1500, answerUnsaved)) asks++;
        const int asked = calls.property(QStringLiteral("calls")).toInt();
        m_.engine->evaluate(QStringLiteral("(function (pop, n) { pop.guard = n.guard; pop.switchTo = n.switchTo })"))
            .call({ m_.engine->newQObject(popup), calls });
        printf("smoke: QD37 USB pick during a hub session: %d asking calls, %d questions shown\n", asked, asks);
        check("QD37: a USB pick during a hub session asks once", asked == 1 && asks == 1);
        check("QD37: and leaves the hub for the USB device", waitFor(5000, [&] {
            return phaseIs("idle") && !b->hubSession() && b->connected() && b->selectedSerial() == QLatin1String(DemoDevice::kSerial);
        }));
        check("QD37: the tooltip offers the reconnect again", waitFor(1000, [&] { return tip().contains(QLatin1String("Right-click to reconnect")); }));
        QObject::disconnect(noted);
        settled();
    }

    // A fixed PIN: the other wording; Cancel leaves the hub
    FakeHubProcess fixed;
    check("QD9: a fixed-PIN hub runs", fixed.start({ "--auth", "fixed:7351" }));
    if (address) address->setProperty("text", fixed.address);
    check("QD9: Connect to it", click(in(popup, QStringLiteral("devicesConnect"))));
    check("QD9: its prompt shows by itself", settled() && waitFor(3000, [&] { return phaseIs("needs_pairing") && opened(dialog); }));
    check("QD9: needs_pairing (fixed) in CG10's words", shows("needs_pairing", "fixed")
                                                              && prompt && prompt->property("text").toString() == cg10Text("needs_pairing", "fixed")
                                                              && cg10Text("needs_pairing", "fixed") != windowText);
    check("QD9: Cancel", click(in(dialog, QStringLiteral("pairingCancel"))));
    check("QD9: Cancel leaves the hub", waitFor(2000, [&] { return phaseIs("idle") && !opened(dialog); }));
    check("QD9: and the demo stays open", b->connected() && b->selectedSerial() == QLatin1String(DemoDevice::kSerial));
    settled();

    // Done closes the list, and browsing is as it was
    check("QD9: Done", click(in(popup, QStringLiteral("devicesDone"))));
    check("QD9: the list closes", waitFor(2000, [&] { return !opened(popup); }));
    check("QD9: browsing back as before", hubs->watching() == watchedBefore);
}
