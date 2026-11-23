// -- WP FEATUI -- The UI for FEAT1-3, against a real hub with an audio link
// installation (`uconsole --demo hub --alink`, DSPI_SMOKE_UCONSOLE):
//  featui-installation: a source on the sender's channels; a unit's own slots;
//    the zone's source switched, its units named and the notice that their
//    hand-made slots are replaced; a room moved into the zone (its unit too); a
//    programme source; a scene that switches the zone's source; Save says how
//    many units take the channels at Align, and Align from there; the hub has
//    it all; the scene recalled says the source; a zone's volume planned and set.
//  featui-ai: the agents' SPL cap and the remembered devices kept across a
//    restart (QSettings) and handed to the server; Settings > This Computer reads.
// Screenshots in <out>/featui/, dark and light.
#include "SmokeDriver.h"

#include "AiController.h"
#include "DeviceModel.h"
#include "HostAudioController.h"
#include "InstallationController.h"
#include "LinkController.h"
#include "ModeController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SystemController.h"

extern "C" {
#include "dspi_core.h"
}

#include <QDir>
#include <QElapsedTimer>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <cstdio>

namespace {

// One `uconsole --demo hub --alink --open` process (as instq's)
class Hub {
public:
    bool start(const QString &store) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_UCONSOLE");
        if (binary.isEmpty()) return false;
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, { "--demo", "hub", "--alink", "--loopback", "--json", "--no-keep-awake", "--forget-clients", "--store", store, "--port", "0",
                               "--open" });
        if (!m_proc.waitForStarted(5000)) return false;
        QElapsedTimer t;
        t.start();
        while (t.elapsed() < 20000) {
            if (!m_proc.canReadLine() && !m_proc.waitForReadyRead(1000)) {
                if (m_proc.state() != QProcess::Running) return false;
                continue;
            }
            while (m_proc.canReadLine()) {
                const QJsonObject e = QJsonDocument::fromJson(m_proc.readLine()).object();
                if (e.value(QLatin1String("event")).toString() == QLatin1String("status")) {
                    url = QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(e.value(QLatin1String("port")).toInt());
                    return true;
                }
            }
        }
        return false;
    }
    ~Hub() {
        if (m_proc.state() == QProcess::NotRunning) return;
        m_proc.kill();
        m_proc.waitForFinished(5000);
    }
    QString url;

private:
    QProcess m_proc;
};

bool click(QQuickItem *item) { return item && item->isVisible() && item->isEnabled() && QMetaObject::invokeMethod(item, "clicked"); }

bool choose(QQuickItem *choice, const QString &id) {
    if (!choice || !choice->isVisible() || !choice->isEnabled()) return false;
    int at = -1, i = 0;
    for (const QVariant &c : choice->property("choices").toList()) {
        if (c.toMap().value(QStringLiteral("id")).toString() == id) at = i;
        ++i;
    }
    if (at < 0) return false;
    choice->setProperty("currentIndex", at);
    return QMetaObject::invokeMethod(choice, "activated", Q_ARG(int, at));
}

bool type(QQuickItem *field, const QString &text) {
    if (!field || !field->isVisible() || !field->isEnabled()) return false;
    field->setProperty("text", text);
    return QMetaObject::invokeMethod(field, "accepted");
}

QString text(QQuickItem *item) { return item && item->isVisible() ? item->property("text").toString() : QString(); }

// The hub's installation as another client reads it
QJsonObject hubModel(const QString &url) {
    FfiCore *other = dspi_core_new();
    if (!other) return {};
    QByteArray buf(256 * 1024, '\0');
    const auto reply = [&](const std::function<uint32_t(char *, uint32_t)> &call) {
        buf.fill('\0');
        call(buf.data(), uint32_t(buf.size()));
        const QJsonObject o = QJsonDocument::fromJson(buf.constData()).object();
        return o.value(QLatin1String("ok")).toBool() ? o : QJsonObject();
    };
    const QByteArray u = url.toUtf8();
    QJsonObject out;
    if (!reply([&](char *b, uint32_t n) { return dspi_link_connect(other, u.constData(), "featui-other", "0", "", "", b, n); }).isEmpty())
        out = reply([&](char *b, uint32_t n) { return dspi_installation_view(other, b, n); })
                  .value(QLatin1String("draft"))
                  .toObject()
                  .value(QLatin1String("model"))
                  .toObject();
    dspi_link_close(other);
    dspi_core_free(other);
    return out;
}

QJsonObject byId(const QJsonObject &model, const char *list, const QString &id) {
    for (const QJsonValue &v : model.value(QLatin1String(list)).toArray())
        if (v.toObject().value(QLatin1String("id")).toString() == id) return v.toObject();
    return {};
}

QJsonArray slotsOf(int a, int b) { return { a, b, 255, 255, 255, 255, 255, 255 }; }

void installationSection(SmokeDriver &d) {
    auto *link = Registrar::instance<LinkController>();
    auto *inst = Registrar::instance<InstallationController>();
    auto *sys = Registrar::instance<SystemController>();
    auto *modes = Registrar::instance<ModeController>();
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    d.check("featui: the controllers", link && inst && sys && modes);
    if (!link || !inst || !sys || !modes) return;
    d.allowSectionTime(240);
    QDir().mkpath(d.outDir() + QStringLiteral("/featui"));
    const bool demoBefore = device->demo() && device->connected();
    const QString modeBefore = modes->mode();
    modes->choose(QStringLiteral("advanced"), false);

    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    QQuickWindow *window = nullptr;
    const auto in = [&](const QString &name) { return window ? d.item(name, window->contentItem()) : nullptr; };
    const auto shoot = [&](const QString &name, int w = 1000, int h = 1300) {
        window->resize(w, h);
        d.spin(200);
        const Qt::ColorScheme before = shell->colorSchemeOverride();
        for (const auto &[scheme, label] : { std::pair{ Qt::ColorScheme::Dark, "dark" }, std::pair{ Qt::ColorScheme::Light, "light" } }) {
            shell->setColorSchemeOverride(scheme);
            d.spin(300);
            d.check(QStringLiteral("featui: screenshot %1-%2").arg(name, QLatin1String(label)),
                    d.grab(window, QStringLiteral("featui/%1-%2.png").arg(name, QLatin1String(label))));
        }
        shell->setColorSchemeOverride(before);
        d.spin(100);
    };
    const auto scrollTo = [&](const QString &name) {
        window->resize(1000, 1300);
        d.spin(200);
        QQuickItem *body = in(QStringLiteral("Installation.body"));
        QQuickItem *item = in(name);
        if (!body || !item) return;
        const qreal y = item->mapToItem(body->property("contentItem").value<QQuickItem *>(), QPointF(0, 0)).y();
        body->setProperty("contentY", qMax<qreal>(0, y - 40));
        d.spin(150);
    };

    Hub hub;
    const bool up = hub.start(d.outDir() + QStringLiteral("/featui/hub"));
    d.check("featui: uconsole --demo hub --alink runs (DSPI_SMOKE_UCONSOLE)", up);
    if (!up) return;
    link->connectToHub(hub.url);
    const bool bound = settled() && d.waitFor(10000, [&] { return link->phase() == QLatin1String("choose_device") && link->devices().size() >= 6; });
    if (bound)
        for (const QVariant &v : link->devices()) {
            const QVariantMap m = v.toMap();
            if (m.value(QStringLiteral("serial")).toString() == QLatin1String("DEMO-ALINK-A"))
                link->bindDevice(m.value(QStringLiteral("handle")).toInt(), m.value(QStringLiteral("serial")).toString());
        }
    d.check("featui: on the hub, DEMO-ALINK-A bound",
            bound && settled() && d.waitFor(10000, [&] { return link->phase() == QLatin1String("bound") && device->connected(); }) && d.waitIdle(20000));
    d.check("featui: the System view is read", d.waitFor(15000, [&] { return sys->available(); }));
    shell->trigger(QStringLiteral("window.system"));
    d.waitFor(3000, [&] {
        window = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("window.system")));
        return window && window->isVisible();
    });
    d.spin(300);
    if (QQuickItem *tabs = in(QStringLiteral("SystemWindow.tabs"))) tabs->setProperty("currentIndex", 1);
    d.check("featui: the Installation tab", d.waitFor(10000, [&] { return inst->shown() && inst->available() && inst->editable(); }));
    if (!window || !inst->editable()) return;

    // ---- What the demo has -------------------------------------------------------
    d.check("featui: the demo's source, its scenes, no programme source", inst->sourceIds() == QStringList{ QStringLiteral("music") }
                                                                           && inst->sceneIds().size() == 4 && inst->programmeIds().isEmpty());
    d.check("featui: Bar's slots belong to the zone playing Music",
            text(in(QStringLiteral("Installation.slotsOwner.D"))) == QLatin1String("Set by Inside, which plays Music")
                && !in(QStringLiteral("Installation.slots.D"))->isEnabled());
    d.check("featui: Terrace's are its own, editable", text(in(QStringLiteral("Installation.slots.C"))) == QLatin1String("2 3 - - - - - -")
                                                           && in(QStringLiteral("Installation.slots.C"))->isEnabled()
                                                           && !in(QStringLiteral("Installation.slotsOwner.C"))->isVisible());
    d.check("featui: the zone names the units it routes",
            text(in(QStringLiteral("Installation.zoneUnits.inside"))) == QLatin1String("Routes Main left, Main right, Bar to its channels."));

    // ---- A source, a unit's slots ----------------------------------------------
    in(QStringLiteral("Installation.new.source"))->setProperty("text", QStringLiteral("Booth"));
    in(QStringLiteral("Installation.newSourceChannels"))->setProperty("text", QStringLiteral("0"));
    d.check("featui: a source on channels Music has is refused, said beside it",
            choose(in(QStringLiteral("Installation.newSourceSender")), QStringLiteral("tx")) && click(in(QStringLiteral("Installation.add.source")))
                && !inst->sourceIds().contains(QStringLiteral("Booth")) && !text(in(QStringLiteral("Installation.newIssue.source"))).isEmpty());
    std::printf("smoke: featui: clash: %s\n", qPrintable(text(in(QStringLiteral("Installation.newIssue.source")))));
    in(QStringLiteral("Installation.newSourceChannels"))->setProperty("text", QStringLiteral("8-9"));
    d.check("featui: a source on channels 8-9 of the sender", click(in(QStringLiteral("Installation.add.source")))
                                                                  && inst->sourceIds().contains(QStringLiteral("Booth")));
    d.spin(200);
    d.check("featui: its card says its channels", text(in(QStringLiteral("Installation.channels.Booth"))) == QLatin1String("8 9"));
    d.check("featui: slots that are not channels are put back", type(in(QStringLiteral("Installation.slots.C")), QStringLiteral("4 x"))
                                                                    && text(in(QStringLiteral("Installation.slots.C"))) == QLatin1String("2 3 - - - - - -"));
    d.check("featui: Terrace's own slots set by hand", type(in(QStringLiteral("Installation.slots.C")), QStringLiteral("4 5"))
                                                           && text(in(QStringLiteral("Installation.slots.C"))) == QLatin1String("4 5 - - - - - -"));
    d.check("featui: no zone source replaces anything yet", inst->rerouted().isEmpty() && !in(QStringLiteral("Installation.rerouted"))->isVisible());

    // ---- The zone's source switched ----------------------------------------------
    d.check("featui: Inside plays Booth", choose(in(QStringLiteral("Installation.zoneSource.inside")), QStringLiteral("Booth"))
                                              && inst->element(QStringLiteral("inside")).value(QStringLiteral("source")).toString() == QLatin1String("Booth"));
    d.spin(200);
    const QString notice = text(in(QStringLiteral("Installation.rerouted")));
    std::printf("smoke: featui: notice: %s\n", qPrintable(notice));
    d.check("featui: the notice says whose own slots are replaced", inst->rerouted().size() == 3
                                                                        && notice.contains(QLatin1String("Main left's own slots will be replaced: Inside plays Booth."))
                                                                        && notice.contains(QLatin1String("Bar's own slots will be replaced")));
    d.check("featui: Bar's slots follow, locked", text(in(QStringLiteral("Installation.slots.D"))) == QLatin1String("8 9 - - - - - -")
                                                      && text(in(QStringLiteral("Installation.slotsOwner.D"))) == QLatin1String("Set by Inside, which plays Booth"));
    // The terrace joins the zone: Terrace's hand-made slots go too
    QQuickItem *member = in(QStringLiteral("Installation.member.inside.terrace"));
    if (member) {
        member->setProperty("checked", true);
        QMetaObject::invokeMethod(member, "toggled");
    }
    d.spin(200);
    d.check("featui: a room moved into the zone: its unit is routed and said", member && inst->rerouted().size() == 4
                                                                                   && text(in(QStringLiteral("Installation.rerouted"))).contains(QLatin1String("Terrace's own slots will be replaced"))
                                                                                   && text(in(QStringLiteral("Installation.slots.C"))) == QLatin1String("8 9 - - - - - -"));
    d.check("featui: and its slots can no longer be set by hand", !in(QStringLiteral("Installation.slots.C"))->isEnabled()
                                                                      && !inst->setSlots(QStringLiteral("C"), { 1, 2, 255, 255, 255, 255, 255, 255 }));

    // ---- A programme source and a scene -----------------------------------------
    in(QStringLiteral("Installation.new.programme"))->setProperty("text", QStringLiteral("Desk computer"));
    in(QStringLiteral("Installation.newProgrammeInputs"))->setProperty("text", QStringLiteral("0 1"));
    d.check("featui: a programme source fed into Main left over USB",
            choose(in(QStringLiteral("Installation.newProgrammeFeeds")), QStringLiteral("A")) && click(in(QStringLiteral("Installation.add.programme")))
                && inst->programmeIds().size() == 1);
    d.spin(200);
    const QString programme = inst->programmeIds().value(0);
    d.check("featui: its card", text(in(QStringLiteral("Installation.inputs.") + programme)) == QLatin1String("0 1")
                                    && inst->element(programme).value(QStringLiteral("programmeKind")).toString() == QLatin1String("computer_usb"));
    in(QStringLiteral("Installation.new.scene"))->setProperty("text", QStringLiteral("DJ"));
    d.check("featui: a scene", click(in(QStringLiteral("Installation.add.scene"))) && inst->sceneIds().contains(QStringLiteral("DJ")));
    d.spin(200);
    d.check("featui: its action switches Inside to Music",
            choose(in(QStringLiteral("Installation.actionTarget.DJ")), QStringLiteral("zone:inside"))
                && choose(in(QStringLiteral("Installation.actionSource.DJ")), QStringLiteral("music"))
                && click(in(QStringLiteral("Installation.addAction.DJ"))));
    d.spin(200);
    d.check("featui: said in words", text(in(QStringLiteral("Installation.action.DJ.0"))) == QStringLiteral("• Switch Inside to source Music"));
    const QString changes = inst->changes().join(QLatin1Char('\n'));
    std::printf("smoke: featui: changes:\n%s\n", qPrintable(changes));
    d.check("featui: the changes list them", changes.contains(QLatin1String("Booth")) && changes.contains(QLatin1String("Desk computer"))
                                                 && changes.contains(QLatin1String("DJ")) && inst->issue().isEmpty());
    scrollTo(QStringLiteral("Installation.slots.C"));
    shoot(QStringLiteral("rerouted"));
    scrollTo(QStringLiteral("Installation.scene.DJ"));
    shoot(QStringLiteral("scene-programme"));
    window->resize(window->minimumWidth(), 900);
    d.spin(300);
    QQuickItem *body = in(QStringLiteral("Installation.body"));
    d.check("featui: at the least width nothing is wider than the panel", body && body->property("contentWidth").toReal() <= body->width());
    d.check("featui: screenshot narrow", d.grab(window, QStringLiteral("featui/narrow.png")));

    // ---- Save, then Align ----------------------------------------------------------
    d.check("featui: Save", click(in(QStringLiteral("Installation.save"))) && d.waitFor(15000, [&] { return !inst->busy() && !inst->dirty(); })
                                && !inst->messageFailed());
    std::printf("smoke: featui: %s\n", qPrintable(inst->message()));
    d.check("featui: saved: 4 units play the new channels after Align", inst->message().contains(QLatin1String("4 unit(s) play the new channels after Align."))
                                                                          && inst->alignPending() == 4 && in(QStringLiteral("Installation.align"))->isVisible());
    const QJsonObject model = hubModel(hub.url);
    d.check("featui: the hub has the source", byId(model, "sources", QStringLiteral("Booth")).value(QLatin1String("channels")).toArray() == QJsonArray{ 8, 9 });
    d.check("featui: the zone plays it, with the terrace", byId(model, "zones", QStringLiteral("inside")).value(QLatin1String("source")).toString() == QLatin1String("Booth")
                                                              && byId(model, "zones", QStringLiteral("inside")).value(QLatin1String("rooms")).toArray().contains(QStringLiteral("terrace")));
    d.check("featui: the units' slots are routed", byId(model, "units", QStringLiteral("C")).value(QLatin1String("slots")).toArray() == slotsOf(8, 9)
                                                      && byId(model, "units", QStringLiteral("A")).value(QLatin1String("slots")).toArray() == slotsOf(8, 9));
    d.check("featui: the programme source", model.value(QLatin1String("programmeSources")).toArray().size() == 1
                                                && model.value(QLatin1String("programmeSources")).toArray().first().toObject().value(QLatin1String("feeds")).toString() == QLatin1String("A"));
    const QJsonArray actions = byId(model, "scenes", QStringLiteral("DJ")).value(QLatin1String("actions")).toArray();
    d.check("featui: the scene's source action", actions.size() == 1 && actions.first().toObject().value(QLatin1String("op")).toString() == QLatin1String("source")
                                                     && actions.first().toObject().value(QLatin1String("source")).toString() == QLatin1String("music"));
    QObject *question = window->findChild<QObject *>(QStringLiteral("Installation.alignQuestion"));
    d.check("featui: Align asks first", click(in(QStringLiteral("Installation.align"))) && question && question->property("visible").toBool());
    if (question) QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    d.check("featui: aligned from the Installation tab", d.waitFor(15000, [&] { return !sys->busy() && !sys->actionMessage().isEmpty(); })
                                                             && inst->alignPending() == 0 && !in(QStringLiteral("Installation.align"))->isVisible()
                                                             && in(QStringLiteral("Installation.alignMessage"))->isVisible());
    std::printf("smoke: featui: align: %s\n", qPrintable(sys->actionMessage()));
    shoot(QStringLiteral("aligned"), 1280, 900);

    // ---- The scene recalled; a zone's volume -----------------------------------------
    if (QQuickItem *tabs = in(QStringLiteral("SystemWindow.tabs"))) tabs->setProperty("currentIndex", 0);
    d.check("featui: the scene is in the System view", d.waitFor(15000, [&] {
        sys->refresh();
        d.spin(300);
        return in(QStringLiteral("SystemWindow.scene.DJ")) != nullptr;
    }));
    d.check("featui: recalled", click(in(QStringLiteral("SystemWindow.scene.DJ")))
                                    && d.waitFor(15000, [&] { return !sys->busy() && sys->lastRecall().contains(QStringLiteral("message")); }));
    const QString recalled = sys->lastRecall().value(QStringLiteral("message")).toString();
    std::printf("smoke: featui: recall: %s\n", qPrintable(recalled));
    d.check("featui: the recall says the zone's new source", recalled.contains(QLatin1String("Inside plays Music"))
                                                                 && text(in(QStringLiteral("SystemWindow.recallResult"))) == recalled);
    d.check("featui: the volume card lists the rooms and zones", d.waitFor(10000, [&] { return sys->volumeTargets().size() == 4; })
                                                                     && in(QStringLiteral("SystemVolume")) && in(QStringLiteral("SystemVolume"))->isVisible());
    sys->planVolume(QStringLiteral("zone:inside"), false, -30, false);
    d.check("featui: a zone's volume planned, unit by unit", d.waitFor(10000, [&] { return !sys->busy() && !sys->volumePlan().isEmpty(); })
                                                                 && sys->volumePlan().value(QStringLiteral("units")).toList().size() >= 4
                                                                 && !sys->volumeSet() && in(QStringLiteral("SystemVolume.row.0")));
    std::printf("smoke: featui: plan row: %s\n", qPrintable(text(in(QStringLiteral("SystemVolume.row.0")))));
    shoot(QStringLiteral("zone-volume"), 1280, 1300);
    d.check("featui: Set", click(in(QStringLiteral("SystemVolume.set"))) || in(QStringLiteral("SystemVolume.louderQuestion")));
    if (QObject *louder = window->findChild<QObject *>(QStringLiteral("SystemVolume.louderQuestion")); louder && louder->property("visible").toBool())
        QMetaObject::invokeMethod(louder, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    d.check("featui: set, each unit said", d.waitFor(15000, [&] { return !sys->busy() && sys->volumeSet(); }) && !sys->volumeMessage().isEmpty());
    std::printf("smoke: featui: volume: %s\n", qPrintable(sys->volumeMessage()));

    window->close();
    link->closeSession();
    settled();
    d.waitFor(10000, [&] { return !device->remote(); });
    d.waitIdle();
    modes->choose(modeBefore, false);
    if (demoBefore && !device->connected()) device->openDemo();
    d.waitFor(10000, [&] { return !demoBefore || device->connected(); });
    d.waitIdle();
}

void aiSection(SmokeDriver &d) {
    auto *ai = Registrar::instance<AiController>();
    auto *host = Registrar::instance<HostAudioController>();
    auto *settings = Registrar::instance<SettingsController>();
    d.check("featui: the AI and host audio controllers", ai && host && settings);
    if (!ai || !host || !settings) return;
    QDir().mkpath(d.outDir() + QStringLiteral("/featui"));
    // The SPL cap: within its range, kept, handed to the server
    const int capBefore = ai->agentSplCap();
    d.check("featui: the agents' SPL cap is 85 dB SPL by default", capBefore == AiController::kSplCapDefault || QSettings().contains(QStringLiteral("mcp/agentSplCapDb")));
    ai->setAgentSplCap(200);
    d.check("featui: never above the core's default for a speaker", ai->agentSplCap() == AiController::kSplCapMax);
    ai->setAgentSplCap(78);
    d.check("featui: kept as mcp/agentSplCapDb", ai->agentSplCap() == 78 && QSettings().value(QStringLiteral("mcp/agentSplCapDb")).toInt() == 78);
    // Remembered devices across a restart: what the next start reads
    const QByteArray stored = QSettings().value(QStringLiteral("devices/remembered")).toByteArray();
    QSettings().setValue(QStringLiteral("devices/remembered"),
                         QJsonDocument(QJsonArray{ QJsonObject{ { QStringLiteral("device"), QStringLiteral("usb:FEATUI-1") }, { QStringLiteral("name"), QStringLiteral("Desk") } } })
                             .toJson(QJsonDocument::Compact));
    ai->loadRemembered();
    const QVariantList remembered = ai->rememberedDevices();
    d.check("featui: a new start remembers the devices, and they are known to agents",
            !remembered.isEmpty() && remembered.constLast().toMap().value(QStringLiteral("device")).toString() == QLatin1String("usb:FEATUI-1")
                && remembered.constLast().toMap().value(QStringLiteral("name")).toString() == QLatin1String("Desk"));
    ai->forgetRememberedDevices();
    d.check("featui: Forget Devices empties the list", ai->rememberedDevices().isEmpty() && !QSettings().contains(QStringLiteral("devices/remembered")));
    if (!stored.isEmpty()) QSettings().setValue(QStringLiteral("devices/remembered"), stored);

    // Settings > AI Agents shows both; Settings > This Computer reads
    settings->open(QStringLiteral("ai"));
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("edit.settings")));
    const auto item = [&](const char *name) { return w ? d.item(QString::fromLatin1(name), w->contentItem()) : nullptr; };
    d.check("featui: Settings > AI Agents has the cap", w && d.waitFor(3000, [&] { return item("AiAgents.splCap") != nullptr; })
                                                            && item("AiAgents.splCap")->property("label").toString() == QLatin1String("Loudest measurement: 78 dB SPL"));
    d.check("featui: and the remembered devices", item("AiAgents.remembered") && item("AiAgents.forgetDevices"));
    bool found = false;
    for (const QVariant &row : settings->sidebarRows(QStringLiteral("default output")))
        found = found || row.toMap().value(QStringLiteral("id")) == QLatin1String("hostaudio");
    d.check("featui: \"default output\" finds This Computer", found);
    if (w) {
        w->resize(1000, 900);
        if (QQuickItem *g = item("AiAgents.measuring")) {
            QQuickItem *flick = g->parentItem();
            while (flick && !flick->inherits("QQuickFlickable")) flick = flick->parentItem();
            if (flick) flick->setProperty("contentY", g->mapToItem(flick->property("contentItem").value<QQuickItem *>(), QPointF(0, 0)).y());
        }
        d.spin(300);
        d.check("featui: screenshot ai-settings", d.grab(w, QStringLiteral("featui/ai-settings.png")));
    }
    settings->open(QStringLiteral("hostaudio"));
    d.check("featui: This Computer opens", d.waitFor(3000, [&] { return item("HostAudioPage") != nullptr; }));
    host->refresh();
    d.check("featui: it reads (never an error)", d.waitFor(10000, [&] { return !host->busy() && !host->reading().isEmpty(); })
                                                     && host->reading().value(QStringLiteral("ok")).toBool());
    std::printf("smoke: featui: host audio: %s, %d device(s)\n", qPrintable(host->reading().value(QStringLiteral("status")).toString()),
                int(host->reading().value(QStringLiteral("devices")).toList().size()));
    const QVariantList devices = host->reading().value(QStringLiteral("devices")).toList();
    d.check("featui: the demo device is no output of this computer, said why",
            devices.isEmpty() || (devices.constFirst().toMap().value(QStringLiteral("output_device")).toString() == QLatin1String("no")
                                  && item("HostAudio.output.0")
                                  && item("HostAudio.output.0")->property("value").toString().startsWith(QLatin1String("No: "))));
    d.spin(300);
    if (w) d.check("featui: screenshot host-audio", d.grab(w, QStringLiteral("featui/host-audio.png")));
    ai->setAgentSplCap(capBefore);
    if (w) w->close();
}

} // namespace

const SmokeDriver::Section featuiInstallation("featui-installation", 136, &installationSection);
const SmokeDriver::Section featuiAi("featui-ai", 985, &aiSection);
