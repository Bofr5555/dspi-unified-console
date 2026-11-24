// -- WP DL5 -- Modern's shell in the design language (DESREV §5, the DL5 row):
// the preset control as one popover (every slot a row with its state, Recall,
// Save, Copy Here and the slot's menu; the name always the active slot's), the
// status strip's dress (the panel tone for info, a 3 px edge, no line under
// it), the rail's 3 × 16 accent mark, the notification drawer's empty state,
// the device switcher without Done and with list rows, and the AI strip that
// always says something, a grant's Revoke in the AI popover (a fake agent over
// HTTP against the real server, as SmokeOdec's).
#include "SmokeDriver.h"

#include "AiController.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DevicesController.h"
#include "LayoutController.h"
#include "MeasureController.h"
#include "NoticeCenter.h"
#include "PresetModel.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"

extern "C" {
#include "dspi_core.h"
}

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTcpServer>
#include <QTcpSocket>
#include <QTest>
#include <memory>

namespace {

QString textOf(QObject *item) { return item ? item->property("text").toString() : QString(); }

// A button's clicked(), or a ListRow's clicked(int)
bool press(QObject *item) {
    auto *i = qobject_cast<QQuickItem *>(item);
    if (!i || !i->isVisible() || !i->isEnabled()) return false;
    return SmokeDriver::press(i);   // -- INTC --
}

bool opened(QObject *popup) { return popup && popup->property("opened").toBool(); }

// ---- The preset popover ----

void presetPart(SmokeDriver &d) {
    auto *sc = Registrar::instance<SidebarController>();
    DeviceModel *device = d.app().device;
    PresetModel *presets = device->presets();
    const auto active = [&] { return presets->activeSlot(); };
    const auto idle = [&] { return d.waitFor(8000, [&] { return !sc->presetBusy(); }) && d.waitIdle(); };
    if (sc->unsaved()) {
        sc->savePreset();
        idle();
        d.waitFor(3000, [&] { return !sc->unsaved(); });
    }
    QQuickItem *box = d.item(QStringLiteral("TopBar.preset"));
    const auto expected = [&] { return active() >= 0 ? sc->presetTitles().value(active()) : QString(); };
    d.check(QStringLiteral("dl5: the preset control names the active slot"),
            box && !expected().isEmpty() && box->property("shownName").toString() == expected());
    d.check(QStringLiteral("dl5: one control: no slot menu button beside it"), d.item(QStringLiteral("TopBar.presetMenu")) == nullptr);
    if (!box) return;
    QObject *popover = box->property("popup").value<QObject *>();
    d.check(QStringLiteral("dl5: its popover"), popover && popover->objectName() == QLatin1String("PresetPopover"));
    if (!popover) return;

    const int start = active();
    int spare = -1;
    for (int s = 0; s < MAX_PRESETS && spare < 0; ++s)
        if (s != start && !sc->presetOccupied(s)) spare = s;
    d.check(QStringLiteral("dl5: the demo has an empty slot"), start >= 0 && spare >= 0);
    if (start < 0 || spare < 0) return;

    QTest::mouseClick(box->window(), Qt::LeftButton, Qt::NoModifier, box->mapToScene(QPointF(box->width() / 2, box->height() / 2)).toPoint());
    d.check(QStringLiteral("dl5: a click opens the popover"), d.waitFor(1500, [&] { return opened(popover); }));
    d.spin(150);
    auto *list = popover->property("contentItem").value<QQuickItem *>();
    d.check(QStringLiteral("dl5: a row for every slot"), list && list->property("count").toInt() == MAX_PRESETS);
    const auto row = [&](int slot, const char *part = nullptr) {
        QString name = QStringLiteral("PresetPopover.slot.%1").arg(slot);
        if (part) name += QLatin1Char('.') + QLatin1String(part);
        return d.item(name, list);
    };
    if (!list) return;
    const bool pointer = list->hasActiveFocus() && list->property("currentIndex").toInt() == -1;
    if (!pointer)
        std::printf("smoke: dl5: the list: focus %d, current %d\n", int(list->hasActiveFocus()), list->property("currentIndex").toInt());
    d.check(QStringLiteral("dl5: opened by the pointer, the list has the keyboard but no row yet"), pointer);
    QQuickItem *activeRow = row(start);
    // -- INTC -- ListRow fades its hidden actions out and keeps them enabled
    // (the DL0 fold on int/B): what shows is the opacity on the way up to the list
    const auto shown = [&](int slot, const char *part) {
        QQuickItem *i = row(slot, part);
        qreal seen = 1;
        for (QQuickItem *p = i; p && p != list; p = p->parentItem()) seen *= p->opacity();
        return i && i->isVisible() && i->isEnabled() && seen > 0;
    };
    d.check(QStringLiteral("dl5: the active row is chosen, says so, and has Save"),
            activeRow && activeRow->property("selected").toBool() && activeRow->property("subtitle").toString().startsWith(QLatin1String("Active"))
                && shown(start, "save") && !shown(start, "recall") && shown(start, "menu"));
    d.check(QStringLiteral("dl5: an empty row says so quietly and offers Copy Here"),
            row(spare) && row(spare)->property("title").toString().isEmpty() && row(spare, "copy") && !shown(spare, "recall"));
    if (QQuickWindow *w = d.mainWindow()) d.grab(w, QStringLiteral("dl5-preset-popover.png"));

    // Copy Here: the spare slot gets the active preset, the active one stays, the popover too.
    // A row's actions show on hover, on the active row and on the keyboard's row
    d.check(QStringLiteral("dl5: an empty row's actions wait for the pointer or the keyboard"), !shown(spare, "copy"));
    QTest::keyClick(list->window(), Qt::Key_Down);
    d.check(QStringLiteral("dl5: Down takes the first row"), list->property("currentIndex").toInt() == 0);
    list->setProperty("currentIndex", spare);
    d.spin(50);
    d.check(QStringLiteral("dl5: on the keyboard's row they show"),
            list->hasActiveFocus() && d.waitFor(1000, [&] { return shown(spare, "copy"); }));   // -- INTC -- after the fade starts
    press(row(spare, "copy"));
    d.check(QStringLiteral("dl5: Copy Here fills the empty slot"),
            d.waitFor(6000, [&] { return sc->presetOccupied(spare) && !sc->presetBusy(); }) && active() == start && opened(popover));
    d.spin(100);
    // Recall: the popover makes way, the slot loads, the name follows
    d.check(QStringLiteral("dl5: its row now offers Recall"), d.waitFor(1000, [&] { return shown(spare, "recall"); }));
    press(row(spare, "recall"));
    d.check(QStringLiteral("dl5: Recall closes the popover and loads the slot"),
            !opened(popover) && d.waitFor(8000, [&] { return active() == spare; }) && idle());
    d.check(QStringLiteral("dl5: the control names it"), d.waitFor(2000, [&] { return box->property("shownName").toString() == expected(); }));
    // From the keyboard: Enter on the control opens it on the active slot's row
    box->forceActiveFocus(Qt::TabFocusReason);
    QTest::keyClick(box->window(), Qt::Key_Return);
    d.check(QStringLiteral("dl5: Enter opens it with the keyboard on the active slot"), d.waitFor(1500, [&] { return opened(popover); })
            && list->hasActiveFocus() && list->property("currentIndex").toInt() == spare);
    QTest::keyClick(list->window(), Qt::Key_Escape);
    d.check(QStringLiteral("dl5: Escape closes it"), d.waitFor(1500, [&] { return !opened(popover); }));
    // The menu of a slot: the slot menu, the popover making way
    QObject *menus = d.app().engine->singletonInstance<QObject *>("UConsole", "ContextMenus");
    QObject *slotMenu = menus ? menus->findChild<QObject *>(QStringLiteral("ContextMenus.slot")) : nullptr;
    QMetaObject::invokeMethod(popover, "open");
    d.waitFor(1500, [&] { return opened(popover); });
    d.spin(150);
    press(row(spare, "menu"));   // the active row's: its actions show
    d.check(QStringLiteral("dl5: a row's menu button opens that slot's menu"),
            d.waitFor(1500, [&] { return opened(slotMenu); }) && !opened(popover));
    if (QQuickWindow *w = d.mainWindow()) d.grab(w, QStringLiteral("dl5-preset-menu.png"));
    if (slotMenu) QMetaObject::invokeMethod(slotMenu, "close");
    d.spin(100);

    // As it was
    sc->requestPresetSwitch(start);
    d.waitFor(8000, [&] { return active() == start; });
    idle();
    sc->clearPreset(spare);
    idle();
    d.check(QStringLiteral("dl5: the demo is back on its slot"), active() == start && !sc->presetOccupied(spare));
}

// ---- The strip, the rail, the drawer, the switcher ----

void shellPart(SmokeDriver &d, QObject *theme) {
    // The demo banner in the strip's dress
    QQuickItem *strip = d.item(QStringLiteral("StatusStrip"));
    QQuickItem *demo = strip ? d.item(QStringLiteral("DemoBanner"), strip) : nullptr;
    QQuickItem *edge = strip ? d.item(QStringLiteral("DemoBanner.edge"), strip) : nullptr;
    d.check(QStringLiteral("dl5: the demo banner is in the strip"), demo && demo->isVisible());
    d.check(QStringLiteral("dl5: on the panel tone, not the blue fill"),
            demo && demo->property("color").value<QColor>() == theme->property("panel").value<QColor>());
    bool edgeOk = false, lineCovered = false;
    if (edge) {
        for (QQuickItem *c : edge->childItems()) {
            const QColor color = c->property("color").value<QColor>();
            if (qAbs(c->width() - 3) < 0.5 && qAbs(c->height() - demo->height()) < 0.5 && c->x() == 0) edgeOk = true;
            if (qAbs(c->height() - 1) < 0.5 && color == demo->property("color").value<QColor>()) lineCovered = true;
        }
    }
    d.check(QStringLiteral("dl5: a 3 px edge at its left"), edgeOk);
    d.check(QStringLiteral("dl5: and no line under it"), lineCovered);
    QQuickItem *aiEdge = d.item(QStringLiteral("AiWorkBanner.edge"));
    d.check(QStringLiteral("dl5: the AI strip's edge is 3 px in Modern"), aiEdge && qAbs(aiEdge->width() - 3) < 0.5);

    // The rail's mark
    d.goTo(QStringLiteral("overview"));
    QQuickItem *mark = d.item(QStringLiteral("NavRail.overview.mark"));
    QQuickItem *other = d.item(QStringLiteral("NavRail.equaliser.mark"));
    d.check(QStringLiteral("dl5: the current rail item has the 3 × 16 accent mark"),
            mark && mark->isVisible() && qAbs(mark->width() - 3) < 0.5 && qAbs(mark->height() - 16) < 0.5
                && mark->property("color").value<QColor>() == theme->property("accent").value<QColor>());
    d.check(QStringLiteral("dl5: the others have none"), other && !other->isVisible());

    // The drawer: empty, then a notice
    auto *notices = Registrar::instance<NoticeCenter>();
    QQuickItem *drawer = d.item(QStringLiteral("NotificationDrawer"));
    if (QQuickItem *clear = d.item(QStringLiteral("NotificationDrawer.clear")); clear && clear->isVisible()) press(clear);
    if (notices) notices->clear();
    d.spin(100);
    if (drawer) drawer->setProperty("open", true);
    d.spin(200);
    QQuickItem *empty = d.item(QStringLiteral("NotificationDrawer.empty"));
    const bool nothing = drawer && drawer->property("waiting").toInt() == 0;
    if (nothing)
        d.check(QStringLiteral("dl5: an empty drawer shows the empty state"), empty && empty->isVisible() && !textOf(empty).isEmpty());
    else
        d.notRun(QStringLiteral("dl5: an empty drawer shows the empty state"), QStringLiteral("a banner waits in the drawer"));
    if (QQuickWindow *w = d.mainWindow()) d.grab(w, QStringLiteral("dl5-drawer-empty.png"));
    if (notices) notices->post(QStringLiteral("DL5 notice"), QStringLiteral("warning"));
    d.check(QStringLiteral("dl5: a notice takes its place"), d.waitFor(1500, [&] { return empty && !empty->isVisible(); }));
    if (drawer) drawer->setProperty("open", false);
    if (notices) notices->clear();
    d.spin(100);

    // The switcher: no Done, list rows, the hubs a row that goes to their page
    auto *list = Registrar::instance<DevicesController>();
    QObject *switcher = d.object(QStringLiteral("DeviceSwitcher"));
    d.app().shell->trigger(QStringLiteral("device.devices"));
    d.check(QStringLiteral("dl5: Devices… opens the switcher"), d.waitFor(1500, [&] { return opened(switcher); }));
    d.spin(150);
    QQuickItem *content = switcher ? switcher->property("contentItem").value<QQuickItem *>() : nullptr;
    QQuickItem *demoRow = content ? d.item(QStringLiteral("DevicesPopup.demo"), content) : nullptr;
    QQuickItem *hubs = content ? d.item(QStringLiteral("DeviceSwitcher.hubs"), content) : nullptr;
    QQuickItem *done = content ? d.item(QStringLiteral("DevicesPopup.done"), content) : nullptr;
    d.check(QStringLiteral("dl5: no Done in the switcher"), content && !done);
    d.check(QStringLiteral("dl5: the demo is a list row, chosen while it is open"),
            demoRow && demoRow->property("rowIndex").isValid() && demoRow->property("selected").toBool() == d.app().device->demo());
    d.check(QStringLiteral("dl5: the device actions stay"), content && d.item(QStringLiteral("DeviceSwitcher.device.reconnect"), content));
    if (QQuickWindow *w = d.mainWindow()) d.grab(w, QStringLiteral("dl5-switcher.png"));
    QTest::keyClick(d.mainWindow(), Qt::Key_Escape);
    d.check(QStringLiteral("dl5: Escape closes it"), d.waitFor(1500, [&] { return !opened(switcher); }));
    d.app().shell->trigger(QStringLiteral("device.devices"));
    d.waitFor(1500, [&] { return opened(switcher); });
    d.check(QStringLiteral("dl5: the hubs' row goes to Devices › Hubs"), press(hubs) && d.waitFor(1500, [&] {
        QObject *host = d.app().shell->pageHost();
        return host && host->property("section").toString() == QLatin1String("devices") && host->property("tab").toString() == QLatin1String("hubs");
    }) && !opened(switcher));
    d.goTo(QStringLiteral("overview"));
    d.waitFor(1000, [&] { return list && !list->listShown(); });
}

// ---- The AI strip: never empty; a grant's Revoke in the AI popover ----

int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// One tool call of the fake agent's (as SmokeOdec's)
struct Call {
    QTcpSocket socket;
    QByteArray in;
    bool done = false;

    Call(int port, const QByteArray &token, const char *tool, const QJsonObject &args) {
        QObject::connect(&socket, &QTcpSocket::readyRead, &socket, [this] { in += socket.readAll(); });
        QObject::connect(&socket, &QTcpSocket::disconnected, &socket, [this] {
            in += socket.readAll();
            done = true;
        });
        QObject::connect(&socket, &QTcpSocket::errorOccurred, &socket, [this] { done = true; });
        const QJsonObject meta{ { QStringLiteral("io.modelcontextprotocol/protocolVersion"), QStringLiteral("2026-07-28") },
                                { QStringLiteral("io.modelcontextprotocol/clientCapabilities"), QJsonObject() } };
        const QJsonObject body{ { QStringLiteral("jsonrpc"), QStringLiteral("2.0") },
                                { QStringLiteral("id"), 1 },
                                { QStringLiteral("method"), QStringLiteral("tools/call") },
                                { QStringLiteral("params"), QJsonObject{ { QStringLiteral("name"), QLatin1String(tool) },
                                                                         { QStringLiteral("arguments"), args },
                                                                         { QStringLiteral("_meta"), meta } } } };
        const QByteArray json = QJsonDocument(body).toJson(QJsonDocument::Compact);
        socket.connectToHost(QStringLiteral("127.0.0.1"), quint16(port));
        socket.write("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAuthorization: Bearer " + token +
                     "\r\nMCP-Protocol-Version: 2026-07-28\r\nMcp-Method: tools/call\r\nMcp-Name: " + QByteArray(tool) +
                     "\r\nContent-Length: " + QByteArray::number(json.size()) + "\r\n\r\n" + json);
    }
};

void aiPart(SmokeDriver &d) {
    auto *ai = Registrar::instance<AiController>();
    auto *mc = Registrar::instance<MeasureController>();
    d.check(QStringLiteral("dl5: the AI and Measure controllers"), ai && mc);
    if (!ai || !mc) return;
    const QString mode = ai->confirmMode();
    const bool capOn = ai->agentSplCapOn();
    const bool followWas = ai->followAgent();
    const int port = freePort();
    ai->setPort(port);
    ai->setConfirmMode(QStringLiteral("ask_risky"));
    ai->setAgentSplCapOn(false);   // the simulated microphone reads no dB SPL
    ai->setFollowAgent(false);
    ai->setEnabled(true);
    d.check(QStringLiteral("dl5: the AI server runs"), d.waitFor(5000, [&] { return ai->running() && !ai->busy(); }));
    ai->addClient(QStringLiteral("Dl5 agent"));
    d.waitFor(3000, [&] { return !ai->newClient().isEmpty(); });
    const QByteArray token = ai->newClient().value(QStringLiteral("token")).toString().toUtf8();
    ai->clearNewClient();
    const auto call = [&](const char *tool, const QJsonObject &args = {}) { return std::make_unique<Call>(port, token, tool, args); };

    // A confirmed speaker setup, so that the agent may measure
    DeviceModel *device = d.app().device;
    device->write(QLatin1String("dl5 volume"), [](FfiCore *c) { return dspi_set_user_volume(c, -20.0f); });
    d.waitIdle();
    const QByteArray layoutJson = QJsonDocument(QJsonObject{
        { QStringLiteral("outputs"), QJsonArray{ QJsonObject{ { QStringLiteral("output"), 0 }, { QStringLiteral("role"), QStringLiteral("low") },
                                                             { QStringLiteral("speaker"), QStringLiteral("L") }, { QStringLiteral("group"), 0 } } } },
        { QStringLiteral("confirmedByUser"), true } }).toJson(QJsonDocument::Compact);
    bool set = false;
    d.app().core->worker()->call(&d, [layoutJson](FfiCore *c) {
        QByteArray buf(1 << 16, '\0');
        return dspi_speaker_layout_set(c, layoutJson.constData(), buf.data(), uint32_t(buf.size())) > 0;
    }, [&](bool) { set = true; });
    d.waitFor(5000, [&] { return set; });

    // A test signal asked and allowed: a grant of ten minutes
    auto held = call("dspi_measure_start", { { QStringLiteral("outputs"), QJsonArray{ 0 } }, { QStringLiteral("sweep_ms"), 5000 },
                                             { QStringLiteral("passes"), 8 } });
    d.check(QStringLiteral("dl5: the agent's measurement is asked"), d.waitFor(5000, [&] { return !ai->confirmations().isEmpty(); }));
    if (ai->confirmations().isEmpty()) return;
    ai->answer(ai->confirmations().value(0).toMap().value(QStringLiteral("id")).toDouble(), true);
    d.check(QStringLiteral("dl5: allowed, a grant"), d.waitFor(3000, [&] { return !ai->grants().isEmpty(); }));
    QQuickItem *strip = d.item(QStringLiteral("AiWorkBanner"));
    QQuickItem *text = d.item(QStringLiteral("AiWorkBanner.text"));
    QQuickItem *grant = d.item(QStringLiteral("AiWorkBanner.grant"));
    QQuickItem *revoke = d.item(QStringLiteral("AiWorkBanner.revokeGrant"));
    QQuickItem *details = d.item(QStringLiteral("AiWorkBanner.details"));
    d.check(QStringLiteral("dl5: while it measures, the grant beside what it does, without Revoke in Modern"), d.waitFor(15000, [&] {
        return mc->measuring() && strip && strip->isVisible() && !textOf(text).isEmpty() && grant && grant->isVisible()
               && revoke && !revoke->isVisible() && details && details->isVisible();
    }));
    { auto stop = call("dspi_measure_stop"); d.waitFor(10000, [&] { return stop->done; }); }
    d.waitFor(10000, [&] { return !mc->measuring(); });
    // Once the agent's work is no longer said, the grant keeps the strip: in words
    const QString expected = QStringLiteral("Test signals allowed, ");
    d.check(QStringLiteral("dl5: after the job, the strip still says something: the grant"),
            d.waitFor(AiController::kLingerMs + 5000, [&] { return ai->work().isEmpty(); }) && d.waitFor(1500, [&] {
                return strip->isVisible() && textOf(text).startsWith(expected) && !grant->isVisible()
                       && textOf(d.item(QStringLiteral("AiWorkBanner.agent"))) == QLatin1String("Dl5 agent");
            }));
    if (QQuickWindow *w = d.mainWindow()) d.grab(w, QStringLiteral("dl5-ai-strip-grant.png"));
    // Revoke is in the popover, which the strip opens
    QQuickItem *aiButton = d.item(QStringLiteral("TopBar.ai"));
    QObject *aiPopover = aiButton ? aiButton->property("popover").value<QObject *>() : nullptr;
    d.check(QStringLiteral("dl5: the strip's button opens the AI popover"), press(details) && d.waitFor(1500, [&] { return opened(aiPopover); }));
    d.spin(150);
    if (QQuickWindow *w = d.mainWindow()) d.grab(w, QStringLiteral("dl5-ai-popover-grant.png"));
    d.check(QStringLiteral("dl5: Revoke there ends the grant, and the strip goes"),
            press(d.item(QStringLiteral("AiPopover.grants.0.revoke"))) && d.waitFor(3000, [&] { return ai->grants().isEmpty(); })
                && d.waitFor(3000, [&] { return !strip->isVisible(); }));
    if (aiPopover) QMetaObject::invokeMethod(aiPopover, "close");

    // As it was
    ai->setEnabled(false);
    d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
    ai->setFollowAgent(followWas);
    ai->setConfirmMode(mode);
    ai->setAgentSplCapOn(capOn);
    ai->setPort(AiController::kDefaultPort);
    ai->revoke(ai->clients().value(0).toMap().value(QStringLiteral("id")).toDouble());
}

void dl5Section(SmokeDriver &d) {
    d.allowSectionTime(150);
    QObject *theme = d.app().engine->singletonInstance<QObject *>("UConsole", "Theme");
    d.check(QStringLiteral("dl5: on the demo"), theme && d.waitFor(10000, [&] { return d.app().device->connected(); }) && d.waitIdle());
    if (!theme || !d.app().device->connected()) return;
    d.check(QStringLiteral("dl5: in Modern"), d.switchLayout("modern"));
    // The keyboard's checks need the main window active (an earlier section's window may have been)
    if (QQuickWindow *w = d.mainWindow()) {
        w->requestActivate();
        d.waitFor(2000, [&] { return w->isActive(); });
    }
    presetPart(d);
    shellPart(d, theme);
    aiPart(d);
    d.switchLayout("classic");
    d.waitIdle();
}

const SmokeDriver::Section dl5("dl5-shell", 988, &dl5Section);

} // namespace
