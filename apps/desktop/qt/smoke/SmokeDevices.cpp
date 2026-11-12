// -- WP QP13 -- The device list and the pairing prompt, through the real QML and
// the app's own controllers, on fake hubs (dspi-link-fakehub), with the demo open
// behind them. One list holds the demo and a hub's devices; a found hub is held
// to the id it announced (wrong_hub); the prompt comes up by itself, does not hold
// the app up, sends six digits or nothing, takes a wrong PIN and a right one;
// every phase shows the core's sentence, both PIN sources and reconnecting's
// button included; the bound device is the app's, a rescan or reconnect leaves it
// alone, polls slow down; the lock notice; choosing the demo leaves the hub;
// Cancel leaves the hub. Adapted from the previous desktop app's QD9 checks.
#include "SmokeDriver.h"

#include "Core.h"
#include "DeviceModel.h"
#include "DevicesController.h"
#include "LinkController.h"
#include "LinkFakeHub.h"
#include "Registrar.h"
#include "Shell.h"

#include <QCoreApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTimer>
#include <QVariantMap>
#include <cstdio>

namespace {
using namespace linksmoke;

// The core's sentence for a phase: what the list must show
QString coreText(const char *phase, const char *pinSource = "") {
    char buf[2048];
    const uint32_t needed = dspi_link_phase_text(phase, pinSource, buf, sizeof(buf));
    if (needed == 0 || needed > sizeof(buf)) return QString();
    return QJsonDocument::fromJson(QByteArray(buf, int(needed) - 1)).object().value(QStringLiteral("text")).toString();
}

bool click(QQuickItem *item) {
    return item && item->isVisible() && item->isEnabled() && QMetaObject::invokeMethod(item, "clicked");
}

void typeInto(QQuickItem *field, const QString &keys) {
    for (const QChar c : keys) {
        const int key = c == QLatin1Char(' ') ? int(Qt::Key_Space) : int(c.toUpper().unicode());
        QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier, QString(c));
        QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier, QString(c));
        QCoreApplication::sendEvent(field, &press);
        QCoreApplication::sendEvent(field, &release);
    }
}

void devicesSection(SmokeDriver &d) {
    auto *link = Registrar::instance<LinkController>();
    auto *list = Registrar::instance<DevicesController>();
    DeviceModel *device = d.app().device;
    QQuickItem *host = list ? list->host() : nullptr;
    QObject *popup = host ? host->findChild<QObject *>(QStringLiteral("DevicesPopup")) : nullptr;
    QObject *dialog = host ? host->findChild<QObject *>(QStringLiteral("PairingDialog")) : nullptr;
    d.check("qp13: the controllers, the list and the prompt are loaded", link && list && popup && dialog);
    if (!link || !list || !popup || !dialog) return;
    FakeHubProcess hub;
    const QString pin = QStringLiteral("482915");
    const bool hubUp = hub.start({ "--rta", "--auth", QStringLiteral("window:") + pin });
    d.check("qp13: the fake hub runs (DSPI_SMOKE_FAKEHUB)", hubUp);
    if (!hubUp) return;

    QQuickWindow *main = d.mainWindow();
    const auto grab = [&](const char *name) { d.grab(main, QStringLiteral("window-devices-%1.png").arg(QLatin1String(name))); };
    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    const auto phaseIs = [link](const char *phase) { return link->phase() == QLatin1String(phase); };
    const auto opened = [](QObject *p) { return p->property("opened").toBool(); };
    // A popup's items: its content, header and footer sit in its popup item
    const auto in = [&](QObject *p, const QString &name) {
        auto *content = p->property("contentItem").value<QQuickItem *>();
        QQuickItem *scope = content && content->parentItem() ? content->parentItem() : content;
        return scope ? d.item(name, scope) : nullptr;
    };
    const auto row = [&](const char *prefix, const QString &key) { return in(popup, QLatin1String(prefix) + key); };
    // The list's status line says the core's sentence for this phase
    const auto shows = [&](const char *phase, const char *pinSource = "") {
        QQuickItem *line = in(popup, QStringLiteral("DevicesPopup.phaseText"));
        const QString want = coreText(phase, pinSource);
        return phaseIs(phase) && !want.isEmpty() && line && line->isVisible() && link->phaseText() == want
               && line->property("text").toString().endsWith(want);
    };
    QObject *poller = d.app().engine->singletonInstance<QObject *>("UConsole", "Poller");
    const bool demoBefore = device->demo() && device->connected();
    d.check("qp13: the demo is open to begin with", demoBefore);

    // Device > Devices... opens it, and it looks for hubs while it shows
    d.check("qp13: idle before", phaseIs("idle") && !list->browseRunning());
    d.app().shell->trigger(QStringLiteral("device.devices"));
    d.check("qp13: Devices... opens the list", d.waitFor(2000, [&] { return opened(popup); }));
    d.check("qp13: and looks for hubs while it shows", list->browseRunning());
    QQuickItem *demoRow = in(popup, QStringLiteral("DevicesPopup.demo"));
    d.check("qp13: the demo is in the list, marked open", demoRow && demoRow->isVisible() && demoRow->property("current").toBool());
    d.check("qp13: idle in the core's words", shows("idle"));
    d.check("qp13: no USB device in this build, and the list says so", list->usbDevices().isEmpty());
    // Find hubs automatically: on by default, remembered, and browsing follows it
    QQuickItem *look = in(popup, QStringLiteral("DevicesPopup.lookForHubs"));
    d.check("qp13: Find hubs automatically is on by default", look && look->property("checked").toBool() && list->lookForHubs());
    list->setLookForHubs(false);
    d.check("qp13: off: remembered, and no browsing", !QSettings().value(QStringLiteral("hub/lookForHubs"), true).toBool()
                                                         && !list->browseRunning() && !look->property("checked").toBool());
    list->setLookForHubs(true);
    d.check("qp13: on again: remembered, and browsing", QSettings().value(QStringLiteral("hub/lookForHubs")).toBool() && list->browseRunning());
    grab("idle");

    // A hub found on the network whose hello names another id: held to the id
    // it announced, so no session and no token
    const QVariantMap spoofed{
        { QStringLiteral("id"), QStringLiteral("00000000-0000-4000-8000-0000000000d9") },
        { QStringLiteral("name"), QStringLiteral("Not That Hub") },
        { QStringLiteral("url"), hub.url },
    };
    list->connectFound(spoofed);
    QQuickItem *error = in(popup, QStringLiteral("DevicesPopup.error"));
    d.check("qp13: a found hub with another id is refused", settled() && link->errorCode() == QLatin1String("wrong_hub"));
    d.check("qp13: the list says so", error && d.waitFor(1000, [&] {
        return error->isVisible() && error->property("text").toString() == link->errorText();
    }));
    d.check("qp13: and no session, no prompt", d.waitFor(2000, [&] { return phaseIs("idle"); }) && !opened(dialog));

    // By address, from the list's own field: the hub asks, the prompt comes up
    QQuickItem *address = in(popup, QStringLiteral("DevicesPopup.address"));
    QQuickItem *connectButton = in(popup, QStringLiteral("DevicesPopup.connect"));
    if (address) address->setProperty("text", QString());
    d.check("qp13: Connect is off with no address", connectButton && !connectButton->isEnabled());
    if (address) address->setProperty("text", hub.address);
    d.check("qp13: Connect by address", click(connectButton));
    d.check("qp13: the hub asks for its PIN", settled() && d.waitFor(3000, [&] { return phaseIs("needs_pairing"); }));
    d.check("qp13: the prompt comes up by itself", d.waitFor(2000, [&] { return opened(dialog); }));
    QQuickItem *prompt = in(dialog, QStringLiteral("PairingDialog.text"));
    const QString windowText = coreText("needs_pairing", "window");
    d.check("qp13: needs_pairing (window) in the core's words, in the prompt and the list",
            shows("needs_pairing", "window") && prompt && prompt->property("text").toString() == windowText);
    d.check("qp13: the prompt names the hub", dialog->property("title").toString().contains(link->hubName()));
    // It does not hold the app up: the event loop runs while it shows
    int ticks = 0;
    QTimer ticker;
    QObject::connect(&ticker, &QTimer::timeout, [&] { ticks++; });
    ticker.start(20);
    d.spin(300);
    ticker.stop();
    d.check("qp13: the prompt does not block", ticks >= 5 && opened(dialog));
    grab("pin");

    // Six digits or nothing: a typo costs no try
    QQuickItem *pinField = in(dialog, QStringLiteral("PairingDialog.pin"));
    QQuickItem *pairButton = in(dialog, QStringLiteral("PairingDialog.pair"));
    QQuickItem *pinError = in(dialog, QStringLiteral("PairingDialog.error"));
    d.check("qp13: Pair is off until a PIN is typed", pairButton && !pairButton->isEnabled());
    if (pinField && pairButton) {
        pinField->forceActiveFocus();
        typeInto(pinField, QStringLiteral("a1 2-345x67"));
        const QString typed = pinField->property("text").toString();
        std::printf("smoke: qp13 typed into the PIN field: '%s'\n", qPrintable(typed));
        d.check("qp13: the PIN field takes six digits and nothing else", typed == QLatin1String("123456") && pairButton->isEnabled());
        for (const char *bad : { "12345", "" }) {
            pinField->setProperty("text", QString::fromLatin1(bad));
            QMetaObject::invokeMethod(dialog, "submit");
            d.check("qp13: Pair is off, and a submit sends nothing, for a PIN that is not six digits",
                    !pairButton->isEnabled() && !link->busy() && link->errorCode().isEmpty() && phaseIs("needs_pairing"));
        }
    }

    // A wrong PIN: the prompt stays, says why, and is empty again
    if (pinField) pinField->setProperty("text", QStringLiteral("000000"));
    d.check("qp13: Pair sends it", click(pairButton));
    d.check("qp13: wrong PIN", settled() && link->errorCode() == QLatin1String("wrong_pin"));
    d.check("qp13: the prompt stays and says so", d.waitFor(1000, [&] {
        return opened(dialog) && pinError && pinError->isVisible() && pinError->property("text").toString() == link->errorText();
    }));
    d.check("qp13: and the list repeats it under the field", error && error->isVisible());
    d.check("qp13: the field is cleared for another try", pinField && pinField->property("text").toString().isEmpty());
    grab("pin-wrong");

    // The right one: past pairing, the hub's device in the same list
    if (pinField) pinField->setProperty("text", pin);
    d.check("qp13: Pair again", click(pairButton));
    d.check("qp13: paired", settled() && link->errorCode().isEmpty());
    d.check("qp13: the prompt goes", d.waitFor(3000, [&] { return !opened(dialog); }));
    d.check("qp13: choose_device in the core's words", d.waitFor(3000, [&] { return shows("choose_device") && link->devices().size() == 1; }));
    const QString serial = link->devices().value(0).toMap().value(QStringLiteral("serial")).toString();
    QQuickItem *hubRow = nullptr;
    d.check("qp13: the hub's device is in the list",
            d.waitFor(2000, [&] { return (hubRow = row("DevicesPopup.hubDevice.", serial)) && hubRow->isVisible(); }));
    d.check("qp13: under the hub's name, its board and firmware",
            hubRow && hubRow->property("subtitle").toString().startsWith(link->hubName() + QStringLiteral(" · RP")));
    // No secret store in this build: the pairing went to the file, and a banner says so
    QQuickItem *tokenNotice = d.item(QStringLiteral("TokenNotice"));
    d.check("qp13: the token notice shows", d.waitFor(3000, [&] { return tokenNotice && tokenNotice->isVisible(); }));
    grab("choose");
    d.check("qp13: its OK dismisses it", click(d.item(QStringLiteral("TokenNotice.ok")))
                                             && d.waitFor(1000, [&] { return !tokenNotice->isVisible(); }));

    // Chosen from the list: bound, the app's device, marked
    d.check("qp13: choose the hub's device", click(hubRow));
    d.check("qp13: bound in the core's words", settled() && d.waitFor(5000, [&] { return shows("bound"); }));
    d.check("qp13: the app shows it", d.waitFor(5000, [&] {
        return device->remote() && device->connected() && device->serial() == serial && !device->demo();
    }));
    d.check("qp13: polls slow down over the hub", poller && poller->property("slowTransport").toBool());
    std::printf("smoke: qp13 device tooltip on the hub: '%s'\n", qPrintable(link->sessionTip().replace(QLatin1Char('\n'), QLatin1String(" | "))));
    d.check("qp13: the device tooltip names the hub and where to choose",
            link->sessionTip().contains(QStringLiteral(" via %1 (%2)").arg(link->hubName(), serial))
                && link->sessionTip().contains(QLatin1String("Device > Devices")));
    // -- WP QPI1 -- the device line in the sidebar carries it
    QQuickItem *statusButton = d.item(QStringLiteral("DeviceStatus.button"));
    QQuickItem *statusText = d.item(QStringLiteral("DeviceStatus.text"));
    d.check("qpi1: the device line's tooltip is the hub session's",
            statusButton && statusButton->property("tip").toString() == link->sessionTip());
    d.check("qp13: marked open, and the demo is not", d.waitFor(1000, [&] {
        return (hubRow = row("DevicesPopup.hubDevice.", serial)) && hubRow->property("current").toBool() && !demoRow->property("current").toBool();
    }));
    grab("bound");

    // A rescan and Reconnect leave the session's device alone
    list->rescan();
    device->reconnect();
    d.app().shell->trigger(QStringLiteral("device.reconnect"));
    d.spin(400);
    d.waitIdle();
    d.check("qp13: a rescan or reconnect during a hub session keeps the hub's device",
            device->remote() && device->connected() && device->serial() == serial && phaseIs("bound"));

    // Another console holds the device's lock: the notice shows until it lets go
    QQuickItem *lock = d.item(QStringLiteral("LockNotice"));
    d.check("qp13: no lock notice at first", lock && !lock->isVisible() && link->lockNotice().isEmpty());
    d.check("qp13: the hub takes lock 77 5000", hub.fault("lock 77 5000"));
    d.check("qp13: the lock notice shows", d.waitFor(3000, [&] { return lock->isVisible() && !link->lockNotice().isEmpty(); }));
    d.check("qpi1: the device line reads the lock notice",
            statusText && statusText->property("text").toString() == link->lockNotice());
    d.check("qpi1: and the device button's notice is the lock's (its tooltip leads with it)",
            statusButton && statusButton->property("notice").toString() == link->lockNotice());
    grab("lock-notice");
    d.check("qp13: the hub takes unlock", hub.fault("unlock"));
    d.check("qp13: the notice goes when the lock does", d.waitFor(3000, [&] { return !lock->isVisible(); }));
    d.check("qpi1: and the device line names the device again", d.waitFor(1000, [&] {
        return statusText && statusText->property("text").toString() == device->deviceTitle();
    }));

    // no_device: the hub's board goes, and comes back
    d.check("qp13: the hub takes board off", hub.fault("board off"));
    d.check("qp13: no_device in the core's words", d.waitFor(5000, [&] { return shows("no_device"); }));
    d.check("qp13: the hub takes board on", hub.fault("board on"));
    d.check("qp13: the device is back", d.waitFor(10000, [&] { return phaseIs("bound") || phaseIs("choose_device"); }));
    if (phaseIs("choose_device")) {
        d.check("qp13: choose it again", d.waitFor(2000, [&] { return (hubRow = row("DevicesPopup.hubDevice.", serial)) != nullptr; })
                                              && click(hubRow));
    }
    d.check("qp13: bound again", settled() && d.waitFor(5000, [&] { return phaseIs("bound"); }));

    // reconnecting, with the core's button and a retry
    d.check("qp13: the hub takes refuse 503", hub.fault("refuse 503"));
    d.check("qp13: the hub takes drop", hub.fault("drop"));
    QQuickItem *action = in(popup, QStringLiteral("DevicesPopup.phaseAction"));
    QQuickItem *retry = in(popup, QStringLiteral("DevicesPopup.retry"));
    d.check("qp13: reconnecting in the core's words", d.waitFor(3000, [&] { return shows("reconnecting"); }));
    d.check("qp13: with the core's button", action && action->isVisible() && link->actionKey() == QLatin1String("reconnecting.disconnect")
                                                 && action->property("text").toString() == link->actionText());
    d.check("qp13: and Retry Now", retry && retry->isVisible());
    d.check("qp13: the hub's device can't be chosen meanwhile", (hubRow = row("DevicesPopup.hubDevice.", serial)) && !hubRow->isEnabled());
    d.check("qp13: the app keeps the hub's device", device->remote() && device->serial() == serial);
    d.check("qp13: browsing follows the hub while it reconnects", list->browseRunning());
    grab("reconnecting");
    d.check("qp13: the hub takes refuse ok", hub.fault("refuse ok"));
    d.check("qp13: Retry Now", click(retry));
    d.check("qp13: bound after the retry", d.waitFor(15000, [&] { return phaseIs("bound"); }));

    // closed: the hub ends the session for good, and the demo opens again
    d.check("qp13: the hub takes close 4000", hub.fault("close 4000"));
    QStringList phases;
    const bool left = d.waitFor(30000, [&] {
        const QString p = link->phase();
        if (phases.isEmpty() || phases.last() != p) phases << p;
        return p != QLatin1String("bound");
    });
    d.check("qp13: closed in the core's words", left && d.waitFor(5000, [&] { return shows("closed"); }));
    std::printf("smoke: qp13 phases after close 4000: %s\n", qPrintable(phases.join(QStringLiteral(" > "))));
    d.check("qp13: no hub devices listed once closed", d.waitFor(2000, [&] { return !row("DevicesPopup.hubDevice.", serial); }));
    d.check("qp13: the device goes with the session, and the demo is back", d.waitFor(5000, [&] {
        return !device->remote() && device->demo() && device->connected();
    }));
    d.check("qp13: polls back to their rate", poller && !poller->property("slowTransport").toBool());
    d.check("qp13: no hub tooltip off the hub", link->sessionTip().isEmpty());
    d.check("qpi1: the device line's tooltip is the plain one again",
            statusButton && statusButton->property("tip").toString().startsWith(QLatin1String("Connected (")));
    grab("closed");

    // Again by address: the stored token, no prompt this time
    if (address) address->setProperty("text", hub.address);
    d.check("qp13: Connect again", click(connectButton));
    d.check("qp13: no PIN asked the second time",
            settled() && d.waitFor(3000, [&] { return phaseIs("choose_device"); }) && !opened(dialog));

    // refused: the hub turns the reconnect away (4xx)
    d.check("qp13: choose it", d.waitFor(2000, [&] { return (hubRow = row("DevicesPopup.hubDevice.", serial)) != nullptr; })
                                   && click(hubRow) && settled() && d.waitFor(5000, [&] { return phaseIs("bound"); }));
    d.check("qp13: the hub takes refuse 403", hub.fault("refuse 403"));
    d.check("qp13: the hub takes drop again", hub.fault("drop"));
    d.check("qp13: lost", d.waitFor(3000, [&] { return phaseIs("reconnecting") || phaseIs("refused"); }));
    if (phaseIs("reconnecting")) link->retryNow();
    d.check("qp13: refused in the core's words", d.waitFor(15000, [&] { return shows("refused"); }));
    d.check("qp13: and the demo is back", d.waitFor(5000, [&] { return !device->remote() && device->demo(); }));
    grab("refused");
    d.check("qp13: the hub takes refuse ok again", hub.fault("refuse ok"));

    // Choosing the demo leaves the hub, which owns the connection while it is up
    if (address) address->setProperty("text", hub.address);
    d.check("qp13: Connect a third time", click(connectButton));
    d.check("qp13: on the hub", settled() && d.waitFor(3000, [&] { return phaseIs("choose_device"); }));
    const uint64_t generationBefore = device->snapshot().generation;
    d.check("qp13: choose the demo", click(demoRow));
    d.check("qp13: the demo is open again and the hub left", d.waitFor(5000, [&] {
        return phaseIs("idle") && d.waitIdle() && device->connected() && device->demo()
               && device->snapshot().generation != generationBefore && demoRow->property("current").toBool();
    }));
    settled();

    // A fixed PIN: the other wording; Cancel leaves the hub
    FakeHubProcess fixed;
    d.check("qp13: a fixed-PIN hub runs", fixed.start({ "--auth", "fixed:7351" }));
    if (address) address->setProperty("text", fixed.address);
    d.check("qp13: Connect to it", click(connectButton));
    d.check("qp13: its prompt comes up by itself", settled() && d.waitFor(3000, [&] { return phaseIs("needs_pairing") && opened(dialog); }));
    d.check("qp13: needs_pairing (fixed) in the core's words",
            shows("needs_pairing", "fixed") && prompt && prompt->property("text").toString() == coreText("needs_pairing", "fixed")
                && coreText("needs_pairing", "fixed") != windowText);
    d.check("qp13: Cancel", click(in(dialog, QStringLiteral("PairingDialog.cancel"))));
    d.check("qp13: Cancel leaves the hub", d.waitFor(2000, [&] { return phaseIs("idle") && !opened(dialog); }));
    d.check("qp13: and the demo stays open", d.waitIdle() && device->connected() && device->demo()
                                                  && dspi_is_connected(d.app().core->handle()));
    settled();

    // Done closes the list, and browsing stops with it
    d.check("qp13: Done", click(in(popup, QStringLiteral("DevicesPopup.done"))));
    d.check("qp13: the list closes", d.waitFor(2000, [&] { return !opened(popup) && !list->listShown(); }));
    d.check("qp13: and stops looking for hubs", d.waitFor(2000, [&] { return !list->browseRunning(); }));
}

const SmokeDriver::Section devices("qp13-devices", 131, &devicesSection);
} // namespace
