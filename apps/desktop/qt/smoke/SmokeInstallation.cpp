// -- WP INSTQ -- Window > System's Installation tab against a real hub with an
// audio link installation: `uconsole --demo hub --alink` (DSPI_SMOKE_UCONSOLE),
// with the app's own LinkController, InstallationController and QML. Another
// client takes two units out of the installation; they are listed as not in it.
// One joins with no feed (said on its field, Save withheld), then with one; the
// other with its room and feed; a zone is made of a room, a room added; the
// changes are listed and saved, and read again by the other client. A loop and a
// removed sender are said on the fields they break. An edit while the other
// client saves: "changed elsewhere", Reload gives the fresh installation (the
// other's change kept); a Save sent before that is seen is the hub's conflict,
// the same. A name with a bidi override is shown without it. Read only, Simple
// and Control offer nothing that writes.
// Screenshots in <out>/instq/, dark and light.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "InstallationController.h"
#include "LinkController.h"
#include "ModeController.h"
#include "Registrar.h"
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
#include <cstdio>

namespace {

// One `uconsole --demo hub --alink` process
class UconsoleHub {
public:
    bool start(const QString &store, const QStringList &extra, bool pairing = false) {
        const QString binary = qEnvironmentVariable("DSPI_SMOKE_UCONSOLE");
        if (binary.isEmpty()) return false;
        m_proc.setProcessChannelMode(QProcess::ForwardedErrorChannel);
        m_proc.start(binary, QStringList{ "--demo", "hub", "--alink", "--loopback", "--json", "--no-keep-awake", "--forget-clients",
                                          "--store", store, "--port", "0" }
                                 + extra);
        if (!m_proc.waitForStarted(5000)) return false;
        QElapsedTimer t;
        t.start();
        int port = 0;
        while (t.elapsed() < 20000) {
            if (!m_proc.canReadLine() && !m_proc.waitForReadyRead(1000)) {
                if (m_proc.state() != QProcess::Running) return false;
                continue;
            }
            while (m_proc.canReadLine()) {
                const QJsonObject e = QJsonDocument::fromJson(m_proc.readLine()).object();
                const QString event = e.value(QLatin1String("event")).toString();
                if (event == QLatin1String("status")) port = e.value(QLatin1String("port")).toInt();
                if (event == QLatin1String("pairing")) pin = e.value(QLatin1String("pin")).toString();
            }
            if (port > 0 && (!pairing || !pin.isEmpty())) {
                url = QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(port);
                return true;
            }
        }
        return false;
    }
    void stop() {
        if (m_proc.state() == QProcess::NotRunning) return;
        m_proc.kill();
        m_proc.waitForFinished(5000);
    }
    ~UconsoleHub() { stop(); }
    QString pin, url;

private:
    QProcess m_proc;
};

bool click(QQuickItem *item) {
    return item && item->isVisible() && item->isEnabled() && QMetaObject::invokeMethod(item, "clicked");
}

// Picks `id` in an InstallationChoice, as a person does
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

QString text(QQuickItem *item) { return item && item->isVisible() ? item->property("text").toString() : QString(); }

// A second Link client on the same hub, through the C ABI: the other installer.
// Its installation; `edits` made and saved first when given. Empty: it failed
QJsonObject elsewhere(const QString &url, const QByteArray &edits = QByteArray()) {
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
    const auto view = [&] { return reply([&](char *b, uint32_t n) { return dspi_installation_view(other, b, n); }); };
    QJsonObject out;
    if (!reply([&](char *b, uint32_t n) { return dspi_link_connect(other, u.constData(), "instq-other", "0", "", "", b, n); }).isEmpty()) {
        out = view();
        if (!edits.isEmpty() && !out.isEmpty()) {
            QByteArray draft = QJsonDocument(out.value(QLatin1String("draft")).toObject()).toJson(QJsonDocument::Compact);
            const QJsonObject edited = reply([&](char *b, uint32_t n) { return dspi_installation_draft_edit(other, draft.constData(), edits.constData(), b, n); });
            draft = QJsonDocument(edited).toJson(QJsonDocument::Compact);
            const bool saved = !edited.isEmpty()
                               && !reply([&](char *b, uint32_t n) { return dspi_installation_save(other, draft.constData(), b, n); }).isEmpty();
            out = saved ? view() : QJsonObject();
        }
    }
    dspi_link_close(other);
    dspi_core_free(other);
    return out;
}

QJsonObject inModel(const QJsonObject &view, const char *list, const QString &key, const QString &value) {
    const QJsonArray all = view.value(QLatin1String("draft")).toObject().value(QLatin1String("model")).toObject().value(QLatin1String(list)).toArray();
    for (const QJsonValue &v : all)
        if (v.toObject().value(key).toString() == value) return v.toObject();
    return {};
}

void installationSection(SmokeDriver &d) {
    auto *link = Registrar::instance<LinkController>();
    auto *inst = Registrar::instance<InstallationController>();
    auto *sys = Registrar::instance<SystemController>();
    auto *modes = Registrar::instance<ModeController>();
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    d.check("instq: the controllers", link && inst && sys && modes);
    if (!link || !inst || !sys || !modes) return;

    const QString bidi = QStringLiteral("a‮b​c d\u0007e") + QString::fromUcs4(U"\U000E0041");
    d.check("instq: a name is shown without control, bidi and invisible characters", InstallationController::clean(bidi) == QLatin1String("abcde"));
    d.check("instq: other text is shown as it is",
            InstallationController::clean(QStringLiteral("Källare ‹1› 寝室 😀")) == QStringLiteral("Källare ‹1› 寝室 😀"));

    d.allowSectionTime(240);
    const QString out = d.outDir() + QStringLiteral("/instq");
    QDir().mkpath(out);
    const bool demoBefore = device->demo() && device->connected();
    const QString modeBefore = modes->mode();
    modes->choose(QStringLiteral("advanced"), false);

    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    const auto phaseIs = [link](const char *phase) { return link->phase() == QLatin1String(phase); };
    const auto bind = [&](const QString &serial) {
        if (!d.waitFor(10000, [&] { return phaseIs("choose_device") && link->devices().size() >= 6; })) return false;
        for (const QVariant &v : link->devices()) {
            const QVariantMap m = v.toMap();
            if (m.value(QStringLiteral("serial")).toString() == serial) link->bindDevice(m.value(QStringLiteral("handle")).toInt(), serial);
        }
        return settled() && d.waitFor(10000, [&] { return phaseIs("bound") && device->remote() && device->connected(); }) && d.waitIdle(20000);
    };
    const auto leave = [&] {
        link->closeSession();
        settled();
        d.waitFor(10000, [&] { return !device->remote(); });
        d.waitIdle();
    };
    QQuickWindow *window = nullptr;
    const auto in = [&](const char *name) { return window ? d.item(QString::fromLatin1(name), window->contentItem()) : nullptr; };
    const auto shoot = [&](const QString &name) {
        window->resize(900, 1300);
        d.spin(200);
        const Qt::ColorScheme before = shell->colorSchemeOverride();
        for (const auto &[scheme, label] : { std::pair{ Qt::ColorScheme::Dark, "dark" }, std::pair{ Qt::ColorScheme::Light, "light" } }) {
            shell->setColorSchemeOverride(scheme);
            d.spin(300);
            d.check(QStringLiteral("instq: screenshot %1-%2").arg(name, QLatin1String(label)),
                    d.grab(window, QStringLiteral("instq/%1-%2.png").arg(name, QLatin1String(label))));
        }
        shell->setColorSchemeOverride(before);
        d.spin(100);
    };
    // The panel's view as the hub has it now
    const auto fresh = [&] {
        inst->refresh();
        d.spin(600);
        inst->refresh();
        d.spin(600);
        return inst->available();
    };
    const auto openTab = [&] {
        shell->trigger(QStringLiteral("window.system"));
        d.waitFor(3000, [&] {
            window = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("window.system")));
            return window && window->isVisible();
        });
        d.spin(300);
        if (QQuickItem *tabs = in("SystemWindow.tabs")) tabs->setProperty("currentIndex", 1);
        return d.waitFor(10000, [&] { return inst->shown() && inst->available(); });
    };

    // ---- An open hub: Admin ----------------------------------------------------
    UconsoleHub hub;
    const bool up = hub.start(out + QStringLiteral("/hub-open"), { QStringLiteral("--open") });
    d.check("instq: uconsole --demo hub --alink runs (DSPI_SMOKE_UCONSOLE)", up);
    if (!up) return;
    // Units B and C are on the hub and not in the installation
    const QJsonObject before = elsewhere(hub.url, R"([{"op":"removeElement","id":"B"},{"op":"removeElement","id":"C"}])");
    d.check("instq: another installer takes B and C out", !before.isEmpty() && inModel(before, "units", "id", "B").isEmpty());
    d.check("instq: nothing is read while not on a hub", !inst->available() && inst->unitIds().isEmpty());
    link->connectToHub(hub.url);
    d.check("instq: on the hub, DEMO-ALINK-A bound", settled() && bind(QStringLiteral("DEMO-ALINK-A")));
    d.check("instq: the System view is read", d.waitFor(15000, [&] { return sys->available(); }));
    d.check("instq: the installation is not read before its tab shows", !inst->shown() && !inst->available());
    d.check("instq: Window > System's Installation tab", openTab());
    if (!window || !inst->available()) return;
    d.check("instq: System is a panel (its PanelTable row)", shell->panel(QStringLiteral("window.system")).value(QStringLiteral("panel")).toString()
                                                                 == QLatin1String("SystemPanel")
                                                             && window->objectName() == QLatin1String("SystemWindow"));
    d.check("instq: the status is the other tab's", in("SystemWindow.units") && !in("SystemWindow.units")->isVisible());
    d.check("instq: Admin on an open hub may edit", inst->mayEdit() && inst->editable() && inst->editNote().isEmpty());
    d.check("instq: the units not in the installation", inst->inventory() && inst->unseenSerials().contains(QStringLiteral("DEMO-ALINK-B"))
                                                            && inst->unseenSerials().contains(QStringLiteral("DEMO-ALINK-C"))
                                                            && !inst->unseenSerials().contains(QStringLiteral("DEMO-ALINK-A")));
    d.check("instq: each has its row", in("Installation.unseen.DEMO-ALINK-B") && in("Installation.unseen.DEMO-ALINK-C")
                                           && !in("Installation.unseen.DEMO-ALINK-A"));
    d.check("instq: nothing to save", !inst->dirty() && in("Installation.save") && !in("Installation.save")->isEnabled());
    shoot(QStringLiteral("unseen"));

    // ---- Joining ---------------------------------------------------------------
    // B with no feed: an invalid upstream, said on its field
    in("Installation.joinName.DEMO-ALINK-B")->setProperty("text", QStringLiteral("Main right"));
    d.check("instq: B joins as a receiver in the main room, no feed",
            choose(in("Installation.joinRoom.DEMO-ALINK-B"), QStringLiteral("main")) && click(in("Installation.join.DEMO-ALINK-B")));
    d.spin(200);
    d.check("instq: B left the list and is a unit", !inst->unseenSerials().contains(QStringLiteral("DEMO-ALINK-B"))
                                                        && inst->unitIds().contains(QStringLiteral("Main right")));
    const QString noFeed = text(in("Installation.upstreamIssue.Main right"));
    std::printf("smoke: instq: no feed: %s | %s\n", qPrintable(noFeed), qPrintable(inst->issue()));
    d.check("instq: an invalid upstream is said on its field", noFeed == QLatin1String("Choose what feeds it."));
    d.check("instq: and Save is withheld", inst->dirty() && !in("Installation.save")->isEnabled() && in("Installation.issue")->isVisible());
    shoot(QStringLiteral("no-feed"));
    inst->save();
    d.check("instq: a Save asked for anyway is not sent", !inst->busy());
    d.check("instq: fed by the fan-out, the issue goes",
            choose(in("Installation.upstream.Main right"), QStringLiteral("fan")) && inst->issue().isEmpty()
                && text(in("Installation.upstreamIssue.Main right")).isEmpty() && in("Installation.save")->isEnabled());
    // C with its room and feed, renamed and given a role after
    d.check("instq: C joins in the terrace, fed by the fan-out",
            choose(in("Installation.joinRoom.DEMO-ALINK-C"), QStringLiteral("terrace"))
                && choose(in("Installation.joinUpstream.DEMO-ALINK-C"), QStringLiteral("fan")));
    in("Installation.joinName.DEMO-ALINK-C")->setProperty("text", QStringLiteral("Deck"));
    d.check("instq: C joined", click(in("Installation.join.DEMO-ALINK-C")) && inst->issue().isEmpty());
    d.spin(200);
    QQuickItem *deckName = in("Installation.name.Deck");
    d.check("instq: a unit is renamed in place", deckName != nullptr);
    if (!deckName) return;
    deckName->setProperty("text", QStringLiteral("Garden deck"));
    QMetaObject::invokeMethod(deckName, "accepted");
    d.spin(100);
    d.check("instq: renamed, its row the same one", in("Installation.name.Deck") == deckName
                                                        && deckName->property("text").toString() == QLatin1String("Garden deck"));

    // ---- Rooms and zones ---------------------------------------------------------
    d.check("instq: a room with no name is refused, said beside it", click(in("Installation.add.room"))
                                                                        && text(in("Installation.newIssue.room")) == QLatin1String("Give it a name."));
    in("Installation.new.room")->setProperty("text", QStringLiteral("Kitchen"));
    d.check("instq: a room is added", click(in("Installation.add.room")) && inst->roomIds().contains(QStringLiteral("Kitchen")));
    d.check("instq: a unit goes into it", choose(in("Installation.room.D"), QStringLiteral("Kitchen"))
                                              && in("Installation.room.D")->property("currentText").toString() == QLatin1String("Kitchen"));
    in("Installation.new.zone")->setProperty("text", QStringLiteral("Outside"));
    d.check("instq: a zone is added, playing the music", choose(in("Installation.newZoneSource"), QStringLiteral("music"))
                                                             && click(in("Installation.add.zone")) && inst->zoneIds().contains(QStringLiteral("Outside")));
    d.spin(200);
    QQuickItem *member = in("Installation.member.Outside.terrace");
    if (member) {
        member->setProperty("checked", true);
        QMetaObject::invokeMethod(member, "toggled");
    }
    d.check("instq: the terrace is in it", member && inst->element(QStringLiteral("Outside")).value(QStringLiteral("rooms")).toStringList()
                                                         == QStringList{ QStringLiteral("terrace") });
    const QString changes = inst->changes().join(QLatin1Char('\n'));
    std::printf("smoke: instq: changes:\n%s\n", qPrintable(changes));
    d.check("instq: the changes are listed", changes.contains(QLatin1String("Adds unit Main right")) && changes.contains(QLatin1String("Adds unit Garden deck"))
                                                 && changes.contains(QLatin1String("Adds room Kitchen")) && changes.contains(QLatin1String("Adds zone Outside"))
                                                 && changes.contains(QStringLiteral("Unit Bar: room Bar → Kitchen"))
                                                 && in("Installation.changes")->property("shownName").toString() == changes);
    shoot(QStringLiteral("changes"));
    // At the window's least width nothing is wider than it; its end, the zones
    window->resize(window->minimumWidth(), 900);
    d.spin(300);
    QQuickItem *body = in("Installation.body");
    if (body) body->setProperty("contentY", body->property("contentHeight").toReal() - body->height());
    d.spin(200);
    d.check("instq: at the least width nothing is cut off", body && body->property("contentWidth").toReal() <= body->width()
                                                                && in("Installation.remove.Outside")->mapToItem(body, QPointF(0, 0)).x()
                                                                           + in("Installation.remove.Outside")->width() <= body->width()
                                                                && in("Installation.add.zone")->mapToItem(body, QPointF(0, 0)).x()
                                                                           + in("Installation.add.zone")->width() <= body->width());
    d.check("instq: screenshot narrow", d.grab(window, QStringLiteral("instq/narrow.png")));
    const int revision = inst->revision();
    d.check("instq: Save", click(in("Installation.save")) && d.waitFor(15000, [&] { return !inst->busy() && !inst->dirty(); })
                               && !inst->messageFailed() && inst->message().startsWith(QLatin1String("Saved on the hub")));
    std::printf("smoke: instq: %s\n", qPrintable(inst->message()));
    d.check("instq: the reread shows them", inst->revision() == revision + 1 && inst->unitIds().contains(QStringLiteral("Main right"))
                                                && !inst->unseenSerials().contains(QStringLiteral("DEMO-ALINK-C")) && inst->changes().isEmpty());
    const QJsonObject after = elsewhere(hub.url);
    const QJsonObject b = inModel(after, "units", "serial", "DEMO-ALINK-B"), c = inModel(after, "units", "serial", "DEMO-ALINK-C");
    d.check("instq: the hub has both, with room and feed", b.value(QLatin1String("room")).toString() == QLatin1String("main")
                                                               && b.value(QLatin1String("upstream")).toString() == QLatin1String("fan")
                                                               && c.value(QLatin1String("name")).toString() == QLatin1String("Garden deck")
                                                               && c.value(QLatin1String("room")).toString() == QLatin1String("terrace"));
    const QJsonObject zone = inModel(after, "zones", "name", "Outside");
    d.check("instq: and the zone", zone.value(QLatin1String("rooms")).toArray() == QJsonArray{ QStringLiteral("terrace") }
                                       && zone.value(QLatin1String("source")).toString() == QLatin1String("music"));

    // ---- Roles, a loop, a removed sender -----------------------------------------
    d.check("instq: the System status tab follows", d.waitFor(15000, [&] {
        sys->refresh();
        for (const QVariant &v : sys->units())
            if (v.toMap().value(QStringLiteral("name")).toString() == QLatin1String("Garden deck")) return true;
        return false;
    }));
    d.check("instq: A is a repeater, and B may be fed by it", choose(in("Installation.role.A"), QStringLiteral("repeater"))
                                                                  && choose(in("Installation.upstream.Main right"), QStringLiteral("A"))
                                                                  && inst->issue().isEmpty());
    d.check("instq: B too, and a loop is said on A's feed", choose(in("Installation.role.Main right"), QStringLiteral("repeater"))
                                                                && choose(in("Installation.upstream.A"), QStringLiteral("Main right")));
    const QString loop = text(in("Installation.upstreamIssue.A"));
    std::printf("smoke: instq: loop: %s\n", qPrintable(loop));
    d.check("instq: the loop in words", loop.contains(QLatin1String("a loop")) && !in("Installation.save")->isEnabled());
    d.check("instq: Discard", click(in("Installation.reload")) && d.waitFor(5000, [&] { return !inst->dirty(); })
                                  && inst->element(QStringLiteral("A")).value(QStringLiteral("role")).toString() == QLatin1String("receiver")
                                  && in("Installation.role.A")->property("currentText").toString() == QLatin1String("Receiver"));
    d.check("instq: a removed sender is said on the fan-out it fed", click(in("Installation.remove.tx")) && d.waitFor(1000, [&] {
        return text(in("Installation.issue.fan")).contains(QLatin1String("is not a sender, fan-out or repeater"));
    }));
    std::printf("smoke: instq: removed: %s\n", qPrintable(text(in("Installation.issue.fan"))));
    d.check("instq: Discard again", click(in("Installation.reload")) && !inst->dirty());
    // An S/PDIF source and its follower, USB-direct
    d.check("instq: D becomes an S/PDIF source with a slot", choose(in("Installation.role.D"), QStringLiteral("spdif_source")) && inst->issue().isEmpty()
                                                                 && inst->element(QStringLiteral("D")).value(QStringLiteral("txMask")).toInt() == 8);
    d.check("instq: never without one", !inst->setSpdifSlot(QStringLiteral("D"), 3, false)
                                            && text(in("Installation.issue.D")).startsWith(QLatin1String("An S/PDIF source")));
    d.check("instq: a follower's feeds are the sources", choose(in("Installation.role.Main right"), QStringLiteral("spdif_follower"))
                                                             && choose(in("Installation.upstream.Main right"), QStringLiteral("D")) && inst->issue().isEmpty());
    d.check("instq: USB-direct has no feed", choose(in("Installation.role.Main right"), QStringLiteral("usb_direct")) && inst->issue().isEmpty()
                                                 && !in("Installation.upstream.Main right")->isVisible());
    d.check("instq: the source becomes a receiver again, to be fed", choose(in("Installation.role.D"), QStringLiteral("receiver"))
                                                                         && !inst->element(QStringLiteral("D")).value(QStringLiteral("spdifSource")).toBool()
                                                                         && text(in("Installation.upstreamIssue.D")) == QLatin1String("Choose what feeds it."));
    d.check("instq: discarded", click(in("Installation.reload")) && !inst->dirty());

    // ---- Changed elsewhere -------------------------------------------------------
    d.check("instq: a view to edit", fresh());
    d.check("instq: an edit", inst->rename(QStringLiteral("main"), QStringLiteral("Great hall")) && inst->dirty() && !inst->stale());
    const QByteArray bidiName = QStringLiteral(R"([{"op":"renameElement","id":"bar","name":"B%1ar"}])").arg(QChar(0x202E)).toUtf8();
    d.check("instq: another installer saves meanwhile", !elsewhere(hub.url, bidiName).isEmpty());
    d.check("instq: the draft is seen stale", fresh() && d.waitFor(8000, [&] { return inst->stale(); }));
    d.check("instq: changed elsewhere: Save withdrawn, Reload offered",
            !in("Installation.save")->isVisible() && text(in("Installation.stale")).contains(QLatin1String("changed elsewhere"))
                && text(in("Installation.reload")) == QLatin1String("Reload") && !inst->editable());
    shoot(QStringLiteral("stale"));
    d.check("instq: Reload gives the fresh installation", click(in("Installation.reload")) && d.waitFor(5000, [&] { return !inst->dirty() && !inst->stale(); })
                                                              && inst->element(QStringLiteral("main")).value(QStringLiteral("name")).toString()
                                                                     == QLatin1String("Main room"));
    d.check("instq: the other's name, without its bidi override",
            inst->element(QStringLiteral("bar")).value(QStringLiteral("name")).toString() == QLatin1String("Bar"));
    d.check("instq: a name shown without it is no edit", inst->rename(QStringLiteral("bar"), QStringLiteral("Bar")) && !inst->dirty());
    d.check("instq: nor in the changes", inst->setRoom(QStringLiteral("D"), QStringLiteral("bar")) && inst->changes().size() == 1
                                             && inst->changes().first() == QStringLiteral("Unit Bar: room Kitchen → Bar"));
    // Sent before the poll sees the change: the hub's conflict
    d.check("instq: the other installer saves again", !elsewhere(hub.url, R"([{"op":"renameElement","id":"main","name":"Foyer"}])").isEmpty());
    d.check("instq: a Save sent stale is the hub's conflict", (inst->save(), true) && d.waitFor(15000, [&] { return !inst->busy() && inst->stale(); })
                                                                  && inst->dirty() && text(in("Installation.stale")).contains(QLatin1String("changed elsewhere")));
    d.check("instq: never saved over", inModel(elsewhere(hub.url), "units", "id", "D").value(QLatin1String("room")).toString() == QLatin1String("Kitchen"));
    d.check("instq: Reload rebuilds the draft of the fresh view", click(in("Installation.reload")) && !inst->stale() && !inst->dirty()
                                                                      && inst->element(QStringLiteral("main")).value(QStringLiteral("name")).toString()
                                                                             == QLatin1String("Foyer"));
    d.check("instq: the edit made again saves", inst->setRoom(QStringLiteral("D"), QStringLiteral("bar")) && (inst->save(), true)
                                                    && d.waitFor(15000, [&] { return !inst->busy() && !inst->dirty(); }) && !inst->messageFailed());
    const QJsonObject both = elsewhere(hub.url);
    d.check("instq: with the other's change kept", inModel(both, "units", "id", "D").value(QLatin1String("room")).toString() == QLatin1String("bar")
                                                       && inModel(both, "rooms", "id", "main").value(QLatin1String("name")).toString() == QLatin1String("Foyer"));

    // ---- Read only on this console, Simple -----------------------------------------
    inst->setReadOnlyForTest(true);
    d.spin(200);
    d.check("instq: read only: nothing offered that writes, and said why",
            !inst->mayEdit() && !inst->addRoom(QStringLiteral("x")) && !inst->dirty() && in("Installation.note")->isVisible()
                && !in("Installation.role.A")->isEnabled() && !in("Installation.remove.A")->isVisible() && !in("Installation.new.room")->isVisible());
    inst->setReadOnlyForTest(false);
    modes->choose(QStringLiteral("simple"), false);
    d.spin(200);
    d.check("instq: Simple shows the status only", !in("SystemWindow.tabs")->isVisible() && in("SystemWindow.units")->isVisible() && !inst->shown());
    modes->choose(QStringLiteral("advanced"), false);
    d.spin(200);
    d.check("instq: Advanced has the tab again", in("SystemWindow.tabs")->isVisible() && inst->shown());

    // Another hub never gets this one's edits; off the hub nothing shows
    d.check("instq: an edit left open", inst->rename(QStringLiteral("main"), QStringLiteral("Left behind")) && inst->dirty());
    window->close();
    d.check("instq: closed, it is not read", !inst->shown());
    leave();
    hub.stop();
    d.check("instq: off the hub, no installation", d.waitFor(5000, [&] { return !inst->available(); }) && !inst->editable());

    // ---- A PIN hub: Control ---------------------------------------------------------
    UconsoleHub pinHub;
    d.check("instq: a PIN hub runs", pinHub.start(out + QStringLiteral("/hub-pin"), { QStringLiteral("--pair") }, true));
    link->connectToHub(pinHub.url);
    d.check("instq: it asks for its PIN", settled() && d.waitFor(5000, [&] { return phaseIs("needs_pairing"); }));
    link->pair(pinHub.pin);
    d.check("instq: paired as Control, DEMO-ALINK-B bound", settled() && bind(QStringLiteral("DEMO-ALINK-B"))
                                                                && d.waitFor(15000, [&] { return sys->available(); }) && openTab());
    d.check("instq: the last hub's edits are not on this one", !inst->dirty()
                                                                   && inst->element(QStringLiteral("main")).value(QStringLiteral("name")).toString()
                                                                          == QLatin1String("Main room"));
    const QString note = text(in("Installation.note"));
    std::printf("smoke: instq: %s: %s\n", qPrintable(inst->role()), qPrintable(note));
    d.check("instq: Control reads, and is told an Admin edits", inst->role() == QLatin1String("control") && !inst->mayEdit()
                                                                    && note.startsWith(QLatin1String("Only an Admin of the hub")));
    d.check("instq: nothing offered that writes", !inst->addRoom(QStringLiteral("x")) && !inst->dirty() && in("Installation.unit.A")
                                                      && !in("Installation.role.A")->isEnabled() && !in("Installation.name.A")->isEnabled()
                                                      && !in("Installation.remove.A")->isVisible() && !in("Installation.new.zone")->isVisible()
                                                      && !in("Installation.save")->isVisible());
    shoot(QStringLiteral("control-role"));
    if (QQuickItem *tabs = in("SystemWindow.tabs")) tabs->setProperty("currentIndex", 0);
    window->close();
    leave();
    pinHub.stop();
    modes->choose(modeBefore, false);
    if (demoBefore && !device->connected()) device->openDemo();
    d.waitFor(10000, [&] { return !demoBefore || device->connected(); });
    d.waitIdle();
}

} // namespace

const SmokeDriver::Section installation("instq-installation", 134, &installationSection);
