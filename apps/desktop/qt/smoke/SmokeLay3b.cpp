// -- WP LAY3b -- Modern's actions, where they act (no menu bar). lay3b-actions:
// every MenuTable row has a place in Modern, and the table as the other clients
// read it goes to <out>/actions.json (run.sh compares it with
// assets/actions.json). lay3b-menus, in Modern on the demo device: every place
// shows its rows; the app menu, Save ▾, Undo ▾ and a device tab's menu; the
// right-click menus of a channel, a band (its row and its dot), the graph and
// a preset slot, and that each row does what it says to the thing under the
// pointer and to nothing else; they close when another device becomes active.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "FirmwareController.h"
#include "GraphController.h"
#include "HistoryController.h"
#include "LayoutController.h"
#include "NoticeCenter.h"
#include "PeqEditorController.h"
#include "PeqOverlay.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "Shell.h"
#include "SidebarController.h"

#include <QCoreApplication>
#include <QFile>
#include <QGuiApplication>
#include <QJSValue>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSet>
#include <QTest>
#include <cmath>
#include <cstdio>

namespace {

bool near(double a, double b, double tol = 0.05) { return std::abs(a - b) <= tol; }

QVariant call(QObject *object, const char *function, const QVariant &arg = {}) {
    QVariant out;
    if (!object) return out;
    if (arg.isValid()) QMetaObject::invokeMethod(object, function, Q_RETURN_ARG(QVariant, out), Q_ARG(QVariant, arg));
    else QMetaObject::invokeMethod(object, function, Q_RETURN_ARG(QVariant, out));
    if (out.canConvert<QJSValue>()) out = out.value<QJSValue>().toVariant();
    return out;
}

// ---- lay3b-actions -------------------------------------------------------------------

void actionsSection(SmokeDriver &d) {
    QObject *table = d.app().engine->singletonInstance<QObject *>("UConsole", "MenuTable");
    const QByteArray text = call(table, "exported").toString().toUtf8();
    const QJsonObject all = QJsonDocument::fromJson(text).object();
    const QJsonArray actions = all.value(QLatin1String("actions")).toArray();
    const QJsonArray items = all.value(QLatin1String("itemActions")).toArray();
    const QJsonObject menus = all.value(QLatin1String("menus")).toObject();
    d.check(QStringLiteral("lay3b: the table is exported"), actions.size() >= 67 && items.size() >= 25 && menus.size() >= 8);

    QSet<QString> known;
    for (const QString &p : table->property("otherPlaces").value<QJSValue>().toVariant().toStringList()) known << p;
    for (auto m = menus.begin(); m != menus.end(); ++m) known << m.key();
    const QVariantMap places = table->property("places").value<QJSValue>().toVariant().toMap();
    const auto placeOk = [&](const QString &place) {
        if (known.contains(place) && !places.contains(place)) return true;
        const QString name = place.section(QLatin1Char('.'), 0, 0), group = place.section(QLatin1Char('.'), 1);
        if (name == QLatin1String("page")) return true;   // a PanelTable row, by MenuTable.homes()
        return places.value(name).toStringList().contains(group);
    };
    QSet<QString> ids;
    for (const QJsonValue &v : actions) {
        const QJsonObject a = v.toObject();
        const QString id = a.value(QLatin1String("id")).toString();
        const QJsonArray where = a.value(QLatin1String("places")).toArray();
        d.check(QStringLiteral("lay3b: %1 has a place in Modern, or is the palette's only").arg(id), !where.isEmpty());
        for (const QJsonValue &p : where)
            d.check(QStringLiteral("lay3b: %1's place %2 is one").arg(id, p.toString()), placeOk(p.toString()));
        ids << id;
    }
    for (const QJsonValue &v : items) {
        const QJsonObject a = v.toObject();
        const QString id = a.value(QLatin1String("id")).toString();
        bool ok = !ids.contains(id) && !a.value(QLatin1String("text")).toString().isEmpty();
        for (const QJsonValue &p : a.value(QLatin1String("places")).toArray()) ok = ok && placeOk(p.toString());
        if (a.contains(QLatin1String("of"))) ok = ok && ids.contains(a.value(QLatin1String("of")).toString());
        d.check(QStringLiteral("lay3b: %1 is an action on one thing, with a place").arg(id), ok);
    }
    // Every row of a place is in its menu, once
    int inMenus = 0, placed = 0;
    for (auto m = menus.begin(); m != menus.end(); ++m)
        for (const QJsonValue &g : m.value().toArray()) inMenus += int(g.toArray().size());
    for (const QJsonArray &list : { actions, items })
        for (const QJsonValue &v : list) {
            const QJsonObject a = v.toObject();
            for (const QJsonValue &p : a.value(QLatin1String("places")).toArray())
                // A row's home on one thing is that other row's place
                if (places.contains(p.toString().section(QLatin1Char('.'), 0, 0))) ++placed;
        }
    int of = 0;
    for (const QJsonValue &v : items)
        if (v.toObject().contains(QLatin1String("of"))) of += int(v.toObject().value(QLatin1String("places")).toArray().size());
    d.check(QStringLiteral("lay3b: the menus hold every placed row once"), inMenus > 0 && inMenus == placed - of);
    d.check(QStringLiteral("lay3b: the app's name is a placeholder"), text.contains("{app}") && !text.contains("uConsole for DSPi"));

    QFile file(d.outDir() + QStringLiteral("/actions.json"));
    d.check(QStringLiteral("lay3b: actions.json is written"), file.open(QIODevice::WriteOnly) && file.write(text) == text.size());
}

// ---- lay3b-menus ---------------------------------------------------------------------

struct Menus {
    SmokeDriver &d;
    Shell *shell;
    LayoutController *layout;
    ChannelController *cc;
    SidebarController *sc;
    PeqEditorController *ed;
    QQuickWindow *main;
    QObject *context;
    int out = -1;   // the first enabled output's wire channel

    void check(const char *what, bool ok) const { d.check(QStringLiteral("lay3b: ") + QString::fromUtf8(what), ok); }
    DeviceModel *device() const { return d.app().device; }
    FilterParams band(int ch, int b) const { return device()->state() ? device()->state()->filters[ch][b] : FilterParams{}; }
    void settle() const {
        d.spin(40);
        d.waitIdle();
        d.spin(80);
    }
    void go(const char *section, const char *tab) const {
        QVariant ok;
        QMetaObject::invokeMethod(shell->pageHost(), "go", Q_RETURN_ARG(QVariant, ok), Q_ARG(QVariant, QString::fromLatin1(section)),
                                  Q_ARG(QVariant, QString::fromLatin1(tab)));
        settle();
    }
    QString section() const { return shell->pageHost() ? shell->pageHost()->property("section").toString() : QString(); }
    // The shown item whose name is `name`, or ends with it ("*.band.0")
    QQuickItem *shown(const QString &name, QQuickItem *from = nullptr) const {
        QQuickItem *at = from ? from : main->contentItem();
        if (!at->isVisible()) return nullptr;
        if (name.startsWith(QLatin1Char('*')) ? at->objectName().endsWith(QStringView(name).mid(1)) : at->objectName() == name) return at;
        for (QQuickItem *c : at->childItems())
            if (QQuickItem *f = shown(name, c)) return f;
        return nullptr;
    }
    QPoint point(QQuickItem *i, qreal fx = 0.5, qreal fy = 0.5) const {
        return i->mapToScene(QPointF(i->width() * fx, i->height() * fy)).toPoint();
    }
    bool click(const QString &name, Qt::MouseButton button = Qt::LeftButton, qreal fx = 0.5) const {
        QQuickItem *i = shown(name);
        if (!i || i->width() <= 0) return false;
        QTest::mouseClick(main, button, Qt::NoModifier, point(i, fx));
        settle();
        return true;
    }
    QObject *menu(const char *name) const {
        const QString n = QString::fromLatin1(name);
        return n.startsWith(QLatin1String("ContextMenus.")) ? context->findChild<QObject *>(n) : d.object(n);
    }
    bool open(const char *name) const {
        QObject *m = menu(name);
        return m && d.waitFor(1500, [&] { return m->property("opened").toBool(); });
    }
    QObject *row(const char *menuName, const char *id) const {
        QObject *m = menu(menuName);
        return m ? m->findChild<QObject *>(QString::fromLatin1(menuName) + QLatin1Char('.') + QString::fromLatin1(id)) : nullptr;
    }
    QString text(const char *menuName, const char *id) const {
        QObject *r = row(menuName, id);
        return r ? r->property("text").toString() : QString();
    }
    bool enabled(const char *menuName, const char *id) const {
        QObject *r = row(menuName, id);
        return r && r->property("enabled").toBool();
    }
    QString keys(const char *menuName, const char *id) const {
        QObject *r = row(menuName, id);
        return r ? r->property("shortcutText").toString() : QStringLiteral("?");
    }
    bool checked(const char *menuName, const char *id) const {
        QObject *r = row(menuName, id);
        return r && r->property("checked").toBool();
    }
    // An open menu's rows are the ones it should have: no line first, last or
    // twice, and as many rows as `rows` (-1: any)
    bool wellFormed(const char *menuName, int rows = -1) const {
        QObject *m = menu(menuName);
        if (!m) return false;
        int seen = 0;
        bool line = true;
        const int count = m->property("count").toInt();
        for (int i = 0; i < count; ++i) {
            QQuickItem *it = nullptr;
            QMetaObject::invokeMethod(m, "itemAt", Q_RETURN_ARG(QQuickItem *, it), Q_ARG(int, i));
            if (!it) return false;
            const bool isLine = it->inherits("QQuickMenuSeparator");
            if (isLine && (line || i == count - 1)) return false;
            line = isLine;
            if (!isLine) ++seen;
        }
        return seen > 0 && (rows < 0 || seen == rows);
    }
    // A click on a row of an open menu, as the pointer makes it
    bool choose(const char *menuName, const char *id) const {
        auto *r = qobject_cast<QQuickItem *>(row(menuName, id));
        if (!r || !r->isVisible() || !r->isEnabled() || r->window() != main) return false;
        QTest::mouseClick(main, Qt::LeftButton, Qt::NoModifier, point(r));
        QObject *m = menu(menuName);
        const bool closed = d.waitFor(1500, [&] { return !m->property("visible").toBool(); });
        settle();
        return closed;
    }
    // A row of a submenu (which a test need not hover open)
    bool chooseSub(const char *menuName, const char *id) const {
        QObject *r = row(menuName, id);
        if (!r) return false;
        QMetaObject::invokeMethod(r, "triggered");
        close(menuName);
        return true;
    }
    void close(const char *menuName) const {
        QObject *m = menu(menuName);
        if (m) QMetaObject::invokeMethod(m, "close");
        d.waitFor(1500, [&] { return !m || !m->property("visible").toBool(); });
        settle();
    }
    bool put(int ch, int b, int type, double hz, double q, double gain) const {
        cc->setBandType(ch, b, false, type);
        cc->setBandValue(ch, b, false, QStringLiteral("freq"), hz);
        cc->setBandValue(ch, b, false, QStringLiteral("q"), q);
        cc->setBandValue(ch, b, false, QStringLiteral("gain"), gain);
        return d.waitFor(4000, [&] { return band(ch, b).filter_type == type && near(band(ch, b).gain, gain); });
    }
    QQuickItem *bandRow(int b) const { return shown(QStringLiteral("*.band.%1").arg(b)); }
    bool bandMenu(int b) const {
        QQuickItem *r = bandRow(b);
        if (!r) return false;
        QTest::mouseClick(main, Qt::RightButton, Qt::NoModifier, point(r, 0.02));
        return open("ContextMenus.band");
    }
    PeqOverlay *overlay() const { return qobject_cast<PeqOverlay *>(shown(QStringLiteral("PeqEditor.overlay"))); }
};

bool switchTo(Menus &m, const QString &layout, const char *shellName) {
    m.layout->setLayout(layout);
    QObject *loader = m.d.object(QStringLiteral("MainWindow.shell"));
    const bool ok = m.d.waitFor(3000, [&] {
        QObject *now = loader ? loader->property("item").value<QObject *>() : nullptr;
        return now && now->objectName() == QLatin1String(shellName);
    });
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    m.d.waitIdle();
    m.d.spin(200);
    return ok;
}

// The menus a button opens: every row of the place, made when it opens
void buttonMenusPart(Menus &m) {
    QObject *table = m.d.app().engine->singletonInstance<QObject *>("UConsole", "MenuTable");
    const QJsonObject menus = QJsonDocument::fromJson(call(table, "exported").toString().toUtf8()).object().value(QLatin1String("menus")).toObject();
    const auto holds = [&](const char *place, const char *menuName) {
        for (const QJsonValue &g : menus.value(QLatin1String(place)).toArray())
            for (const QJsonValue &v : g.toArray()) {
                const QString id = v.toString();
                // An optional row nothing handles is left out; so is an empty list
                const QVariantMap r = call(table, "row", id).toMap();
                if (r.value(QStringLiteral("optional")).toBool() && !m.shell->handles(id)) continue;
                if (r.value(QStringLiteral("dynamic")).toBool()) continue;
                m.d.check(QStringLiteral("lay3b: %1 is in %2").arg(id, QLatin1String(menuName)), m.row(menuName, qPrintable(id)) != nullptr);
            }
    };
    m.click(QStringLiteral("AppMenuButton"));
    m.check("the app button opens its menu", m.open("AppMenu"));
    holds("appMenu", "AppMenu");
    QObject *app = m.menu("AppMenu");
    m.check("it holds only the few global actions", app && app->property("count").toInt() <= 26 && !m.row("AppMenu", "tools.matrix"));
    m.check("in groups with a line between two", m.wellFormed("AppMenu"));
    m.check("and fits the window", app && app->property("height").toReal() <= m.main->height());
    m.check("Save File waits for a file", !m.enabled("AppMenu", "file.saveFile") && m.enabled("AppMenu", "file.openFile"));
    m.check("its rows show the shortcuts", m.keys("AppMenu", "file.importFilters") == QLatin1String("Ctrl+I"));
    m.d.grab(m.main, QStringLiteral("modern-app-menu.png"));
    m.check("a row goes to its page", m.choose("AppMenu", "edit.settings") && m.shell->pageAction() == QLatin1String("edit.settings"));
    m.go("overview", "overview");

    m.check("Save ▾ opens", m.click(QStringLiteral("TopBar.save.more")) && m.open("TopBar.save.menu"));
    holds("save", "TopBar.save.menu");
    m.d.grab(m.main, QStringLiteral("modern-save-menu.png"));
    // -- WP UA3 -- (UXP C1) Factory Reset is not in Save ▾ (ua3-savestate: on Devices)
    m.check("Factory Reset is not in it", !m.row("TopBar.save.menu", "device.factoryReset"));
    m.check("Revert to Saved asks", m.choose("TopBar.save.menu", "device.revert"));
    QObject *question = m.d.object(QStringLiteral("FilesHost.deviceQuestion"));
    m.check("its question shows", question && m.d.waitFor(2000, [&] { return question->property("opened").toBool(); }));
    if (question) QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    m.settle();

    // Undo ▾: what was changed, the newest first
    const double before = m.device()->state()->output_gain_db[0];
    m.cc->setOutputGain(0, before - 2);
    m.settle();
    auto *history = Registrar::instance<HistoryController>();
    m.check("a change is in the history", m.d.waitFor(3000, [&] { return history && history->canUndo() && !history->entries().isEmpty(); }));
    m.check("Undo ▾ opens", m.click(QStringLiteral("TopBar.undo.more")) && m.open("TopBar.undo.menu"));
    holds("history", "TopBar.undo.menu");
    m.check("Undo ▾ is well formed", m.wellFormed("TopBar.undo.menu", 4 + int(qMin(8, int(history->entries().size())))));
    m.check("with the last changes", history && !history->entries().isEmpty()
            && m.text("TopBar.undo.menu", "history.recent.0").startsWith(history->entries().first().toMap().value(QStringLiteral("title")).toString()));
    if (auto *r = qobject_cast<QQuickItem *>(m.row("TopBar.undo.menu", "edit.undo"))) {
        const QRectF at = r->mapRectToScene(QRectF(0, 0, r->width(), r->height()));
        m.check("and lies within the window", at.left() >= 0 && at.right() <= m.main->width() && at.width() >= 200);
    }
    m.d.grab(m.main, QStringLiteral("modern-undo-menu.png"));
    m.check("Undo undoes", m.choose("TopBar.undo.menu", "edit.undo")
            && m.d.waitFor(3000, [&] { return near(m.device()->state()->output_gain_db[0], before); }));
    m.click(QStringLiteral("TopBar.undo.more"));
    m.check("History… is the page", m.open("TopBar.undo.menu") && m.choose("TopBar.undo.menu", "edit.history")
            && m.shell->pageAction() == QLatin1String("edit.history"));
    m.click(QStringLiteral("TopBar.undo.more"));
    m.open("TopBar.undo.menu");
    m.go("overview", "overview");
    m.check("a change's row shows the history", m.choose("TopBar.undo.menu", "history.recent.0") && m.shell->pageAction() == QLatin1String("edit.history"));

    m.shell->trigger(QStringLiteral("autoeq.browse"));
    m.settle();
    QObject *autoeq = m.shown(QStringLiteral("AutoEqWindow.more"));
    m.check("Library › AutoEQ has what its menu had", autoeq != nullptr);
    m.click(QStringLiteral("AutoEqWindow.more"));
    QObject *autoeqMenu = autoeq ? autoeq->findChild<QObject *>(QStringLiteral("AutoEqWindow.menu")) : nullptr;
    m.check("with Update Database and Clear Favorites", autoeqMenu && m.d.waitFor(1500, [&] { return autoeqMenu->property("opened").toBool(); })
            && autoeqMenu->findChild<QObject *>(QStringLiteral("AutoEqWindow.menu.autoeq.updateDatabase"))
            && autoeqMenu->findChild<QObject *>(QStringLiteral("AutoEqWindow.menu.autoeq.clearFavorites")));
    if (autoeqMenu) QMetaObject::invokeMethod(autoeqMenu, "close");
    m.settle();
    // -- WP UA3 -- TopBar.ab shows while a compare runs only (ua3-savestate)
    for (const char *name : { "TopBar.undo.button", "TopBar.redo", "TopBar.save.button", "TopBar.preset", "DeviceTabs.add", "Dock.mute",
                              "Dock.bypass" })
        m.d.check(QStringLiteral("lay3b: %1, a place that is a control, shows").arg(QLatin1String(name)), m.shown(QString::fromLatin1(name)) != nullptr);
    m.go("overview", "overview");
}

// A device tab's menu, and Close while the device is pinned (RB)
void devicePart(Menus &m) {
    auto *registry = DeviceRegistry::instance();
    auto *firmware = Registrar::instance<FirmwareController>();
    DeviceSession *session = registry->active();
    const QString id = registry->activeId();
    const QByteArray name = QByteArray("DeviceTabs.menu.") + id.toUtf8();
    QQuickItem *tab = m.shown(QStringLiteral("DeviceTabs.tab.") + id);
    QObject *menu = tab ? tab->findChild<QObject *>(QString::fromUtf8(name)) : nullptr;
    const auto openMenu = [&] {
        QTest::mouseClick(m.main, Qt::RightButton, Qt::NoModifier, m.point(tab, 0.4));
        return menu && m.d.waitFor(1500, [&] { return menu->property("opened").toBool(); });
    };
    const auto row = [&](const char *which) {
        return menu ? menu->findChild<QObject *>(QString::fromUtf8(name) + QLatin1Char('.') + QLatin1String(which)) : nullptr;
    };
    const auto shut = [&] {
        if (menu) QMetaObject::invokeMethod(menu, "close");
        m.d.waitFor(1500, [&] { return menu && !menu->property("visible").toBool(); });
        m.settle();
    };
    // -- WP LAY7 -- the reason's row is there always, and shows while there is one
    const auto noReason = [&] {
        auto *item = qobject_cast<QQuickItem *>(row("device.close.why"));
        return item && !item->isVisible() && item->height() < 1;
    };
    m.check("a right-click on a device tab opens its menu", tab && openMenu());
    for (const char *which : { "device.identify", "device.reconnect", "device.firmwareUpdate", "device.factoryReset", "device.close" })
        m.d.check(QStringLiteral("lay3b: %1 is in it").arg(QLatin1String(which)), row(which) != nullptr);
    // -- WP UFW -- (UXP A3, C1) Factory Reset is the device's, and asks first
    auto *reset = qobject_cast<QQuickItem *>(row("device.factoryReset"));
    m.check("Factory Reset is there for the shown device", reset && reset->isEnabled());
    if (reset) QTest::mouseClick(m.main, Qt::LeftButton, Qt::NoModifier, m.point(reset));
    m.settle();
    QObject *question = m.d.object(QStringLiteral("FilesHost.deviceQuestion"));
    m.check("Factory Reset asks", question && m.d.waitFor(2000, [&] { return question->property("opened").toBool(); }));
    if (question) QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    m.settle();
    m.check("the tab's menu opens again", openMenu());
    m.check("Close works while nothing pins the device", row("device.close") && row("device.close")->property("enabled").toBool() && noReason());
    shut();
    firmware->pretendRunForTest(session);
    m.d.spin(100);
    m.check("pinned: the menu opens", openMenu());
    QObject *why = row("device.close.why");
    m.check("Close is greyed, with the reason under it", row("device.close") && !row("device.close")->property("enabled").toBool()
            && why && why->property("text").toString().contains(session->title()));
    auto *whyItem = qobject_cast<QQuickItem *>(why);
    m.check("the reason is readable whole", whyItem && whyItem->height() > 30 && whyItem->width() > 200);
    m.d.grab(m.main, QStringLiteral("modern-tab-menu-pinned.png"));
    QTest::mouseClick(m.main, Qt::LeftButton, Qt::NoModifier, m.point(qobject_cast<QQuickItem *>(row("device.close"))));
    m.d.spin(200);
    m.check("a click on it closes nothing", registry->session(id) == session);
    shut();
    firmware->pretendRunForTest(nullptr);
    m.d.spin(100);
    m.check("let go, Close works again", openMenu() && row("device.close")->property("enabled").toBool() && noReason());
    shut();
    // The switcher has the same actions
    m.click(QStringLiteral("DeviceTabs.add"));
    QObject *switcher = m.d.object(QStringLiteral("DeviceSwitcher"));
    m.d.waitFor(1500, [&] { return switcher && switcher->property("opened").toBool(); });
    for (const char *which : { "device.identify", "device.reconnect", "device.firmwareUpdate" })
        m.d.check(QStringLiteral("lay3b: the switcher has %1").arg(QLatin1String(which)), m.shown(QStringLiteral("DeviceSwitcher.") + QLatin1String(which)) != nullptr);
    m.check("and not a tab's Close", !m.shown(QStringLiteral("DeviceSwitcher.device.close")));
    m.check("nor Factory Reset", !m.shown(QStringLiteral("DeviceSwitcher.device.factoryReset")));   // -- WP UFW --
    if (switcher) QMetaObject::invokeMethod(switcher, "close");
    m.settle();
}

void channelPart(Menus &m) {
    const char *menu = "ContextMenus.channel";
    auto *graph = Registrar::instance<GraphController>();
    m.go("equaliser", "peq");
    m.sc->select(0);
    m.settle();
    const QString rowName = QStringLiteral("Sidebar.channel.%1").arg(m.out);
    const int number = m.out - m.cc->numInputs();
    const auto openRow = [&] { return m.click(rowName, Qt::RightButton, 0.3) && m.open(menu); };
    m.check("a right-click on an output opens its menu", openRow());
    m.check("an output's menu is well formed", m.wellFormed(menu));
    m.check("and leaves the selection", m.sc->selectedChannel() == 0);
    for (const char *id : { "channel.rename", "channel.curve", "channel.copy", "channel.paste", "channel.mute", "channel.routing", "channel.clear" })
        m.d.check(QStringLiteral("lay3b: an output's menu has %1").arg(QLatin1String(id)), m.row(menu, id) != nullptr);
    m.check("Identify as the device offers it, no headphone profile",
            (m.row(menu, "channel.identify") != nullptr) == m.sc->identifySupported() && !m.row(menu, "channel.headphone"));
    m.check("Copy shows no shortcut: that copies the selected channel", m.keys(menu, "channel.copy").isEmpty());
    m.d.grab(m.main, QStringLiteral("modern-menu-channel.png"));
    const bool muted = m.cc->output(number).value(QStringLiteral("muted")).toBool();
    m.check("Mute mutes it", m.choose(menu, "channel.mute")
            && m.d.waitFor(3000, [&] { return m.cc->output(number).value(QStringLiteral("muted")).toBool() != muted; }));
    m.check("then the row says Unmute", openRow() && m.text(menu, "channel.mute") == (muted ? QLatin1String("Mute") : QLatin1String("Unmute")));
    m.check("which unmutes it", m.choose(menu, "channel.mute")
            && m.d.waitFor(3000, [&] { return m.cc->output(number).value(QStringLiteral("muted")).toBool() == muted; }));
    const bool curve = graph->isCurveVisible(m.out);
    openRow();
    m.check("the curve's row says what it does", m.text(menu, "channel.curve") == (curve ? QLatin1String("Hide Curve") : QLatin1String("Show Curve")));
    m.check("and does it", m.choose(menu, "channel.curve") && graph->isCurveVisible(m.out) != curve);
    graph->setCurveVisible(m.out, curve);

    // Copy and paste, on the channel under the pointer
    m.put(m.out, 0, FILTER_PEAKING, 777, 1.5, 3.5);
    const FilterParams kept = m.band(m.out + 1, 0);
    openRow();
    m.check("Copy copies it", m.choose(menu, "channel.copy") && m.d.waitFor(2000, [&] { return m.sc->hasClipboard(); }));
    const QString other = QStringLiteral("Sidebar.channel.%1").arg(m.out + 1);
    if (m.shown(other)) {
        m.click(other, Qt::RightButton, 0.3);
        m.check("Paste pastes on the other", m.open(menu) && m.choose(menu, "channel.paste")
                && m.d.waitFor(4000, [&] { return near(m.band(m.out + 1, 0).freq, 777, 1) && near(m.band(m.out + 1, 0).gain, 3.5); }));
        m.put(m.out + 1, 0, kept.filter_type, kept.freq, kept.q, kept.gain);
    }

    openRow();
    m.check("Rename begins in the row", m.choose(menu, "channel.rename") && m.shown(QStringLiteral("Sidebar.rename.%1").arg(m.out)));
    QTest::keyClick(m.main, Qt::Key_Escape);
    m.settle();

    openRow();
    m.check("Clear PEQ asks", m.choose(menu, "channel.clear"));
    QObject *question = m.main->contentItem()->findChild<QObject *>(QStringLiteral("ContextMenus.clearQuestion"));
    m.check("in the window's middle", question && m.d.waitFor(1500, [&] { return question->property("opened").toBool(); }));
    m.d.grab(m.main, QStringLiteral("modern-clear-question.png"));
    if (question) QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    m.settle();
    m.check("refused, the bands stay", m.band(m.out, 0).filter_type == FILTER_PEAKING);
    openRow();
    m.choose(menu, "channel.clear");
    m.d.waitFor(1500, [&] { return question && question->property("opened").toBool(); });
    if (question) QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    m.check("answered, they are flat", m.d.waitFor(4000, [&] { return m.band(m.out, 0).filter_type == FILTER_FLAT; }));
    m.settle();

    openRow();
    m.check("Edit Routing goes to the matrix", m.choose(menu, "channel.routing") && m.shell->pageAction() == QLatin1String("tools.matrix"));
    m.go("equaliser", "peq");
    m.sc->select(0);
    m.settle();
    m.check("an input's menu", m.click(QStringLiteral("Sidebar.channel.0"), Qt::RightButton, 0.3) && m.open(menu));
    m.check("has the headphone profile, and nothing of an output's",
            m.row(menu, "channel.headphone") && !m.row(menu, "channel.mute") && !m.row(menu, "channel.routing") && !m.row(menu, "channel.identify"));
    m.check("the selected channel's Copy shows its shortcut", m.keys(menu, "channel.copy") == QLatin1String("Ctrl+C"));
    m.check("Load Headphone Profile goes to AutoEQ", m.choose(menu, "channel.headphone") && m.shell->pageAction() == QLatin1String("autoeq.browse"));
    m.go("equaliser", "peq");
    // Without the mouse
    m.settle();
    if (QQuickItem *r = m.shown(rowName)) {
        r->forceActiveFocus();
        QTest::keyClick(m.main, Qt::Key_F10, Qt::ShiftModifier);
        m.check("Shift+F10 on a channel opens its menu", m.open(menu));
        m.close(menu);
    }
}

void bandPart(Menus &m) {
    const char *menu = "ContextMenus.band";
    m.sc->select(m.out);
    m.settle();
    m.cc->clearBands(m.out, false);
    m.settle();
    m.put(m.out, 0, FILTER_PEAKING, 300, 1.0, 4.0);
    m.put(m.out, 1, FILTER_PEAKING, 3000, 2.0, -3.0);
    m.settle();
    const auto b = [&](int i) { return m.band(m.out, i); };

    m.check("a right-click on a band's row opens its menu", m.bandMenu(0));
    for (const char *id : { "band.shape", "band.slope", "band.bypass", "band.invert", "band.copy", "band.paste", "band.reset" })
        // A bell has one slope
        m.d.check(QStringLiteral("lay3b: a bell's menu: %1").arg(QLatin1String(id)),
                  (m.row(menu, id) != nullptr) == (QLatin1String(id) != QLatin1String("band.slope")));
    m.check("one row each", m.wellFormed(menu, 6));
    m.check("Paste waits for a copy", !m.ed->hasBand() && !m.enabled(menu, "band.paste"));
    m.check("Shape marks the band's own", m.checked(menu, "band.shape.0") && m.row(menu, "band.shape.1") && !m.checked(menu, "band.shape.1"));
    m.d.grab(m.main, QStringLiteral("modern-menu-band.png"));
    m.check("Invert Gain inverts it, and only it", m.choose(menu, "band.invert") && m.d.waitFor(3000, [&] { return near(b(0).gain, -4); }) && near(b(1).gain, -3));
    m.bandMenu(0);
    m.check("Bypass bypasses it", m.choose(menu, "band.bypass") && m.d.waitFor(3000, [&] { return b(0).bypass != 0; }) && b(1).bypass == 0);
    m.bandMenu(0);
    m.check("then the row says Enable", m.text(menu, "band.bypass") == QLatin1String("Enable"));
    m.check("which enables it", m.choose(menu, "band.bypass") && m.d.waitFor(3000, [&] { return b(0).bypass == 0; }));
    m.bandMenu(1);
    m.check("Copy Band", m.choose(menu, "band.copy") && m.ed->hasBand());
    m.bandMenu(2);
    m.check("an Off band can only be shaped or pasted on", !m.enabled(menu, "band.bypass") && !m.enabled(menu, "band.copy") && !m.enabled(menu, "band.reset")
            && m.enabled(menu, "band.paste") && m.row(menu, "band.shape") && !m.row(menu, "band.invert"));
    m.check("Paste Band makes it the copied one", m.choose(menu, "band.paste")
            && m.d.waitFor(3000, [&] { return b(2).filter_type == FILTER_PEAKING && near(b(2).freq, 3000, 1) && near(b(2).q, 2) && near(b(2).gain, -3); }));
    m.bandMenu(2);
    m.check("Shape reshapes it, keeping its frequency", m.chooseSub(menu, "band.shape.2")
            && m.d.waitFor(3000, [&] { return b(2).filter_type == FILTER_HIGHPASS && near(b(2).freq, 3000, 1) && near(b(2).gain, 0); }));
    if (!m.ed->slopes(FILTER_HIGHPASS).isEmpty()) {
        m.bandMenu(2);
        m.check("a cut's menu has its slopes", m.row(menu, "band.slope") && m.checked(menu, "band.slope.2"));
        m.check("Slope changes it", m.chooseSub(menu, "band.slope.1") && m.d.waitFor(3000, [&] { return b(2).filter_type == FILTER_HIGHPASS1; }));
    }
    m.bandMenu(2);
    m.check("Reset Band resets it", m.choose(menu, "band.reset") && m.d.waitFor(3000, [&] { return b(2).filter_type == FILTER_FLAT; })
            && b(0).filter_type == FILTER_PEAKING);

    // Several: the selection, when the band is one of it
    m.ed->rowClicked(0, 0);
    m.ed->rowClicked(1, Qt::ControlModifier);
    m.settle();
    m.bandMenu(1);
    m.check("a row of the selection: the menu is for all of it", m.text(menu, "band.reset") == QLatin1String("Reset 2 Bands"));
    m.check("Invert Gain inverts them all", m.choose(menu, "band.invert") && m.d.waitFor(3000, [&] { return near(b(0).gain, 4) && near(b(1).gain, 3); }));
    m.put(m.out, 3, FILTER_PEAKING, 9000, 1, 2);
    m.settle();
    m.bandMenu(3);
    m.check("a row outside it: the menu is for that band", m.text(menu, "band.reset") == QLatin1String("Reset Band"));
    m.check("which is selected now", m.ed->selected() == QList<int>{ 3 });
    m.check("and leaves the others", m.choose(menu, "band.reset") && m.d.waitFor(3000, [&] { return b(3).filter_type == FILTER_FLAT; })
            && b(0).filter_type == FILTER_PEAKING && b(1).filter_type == FILTER_PEAKING);
    m.check("a band reset is no longer selected", m.ed->selected().isEmpty());
    m.ed->forget();
    m.settle();

    // Its dot
    PeqOverlay *ov = m.overlay();
    m.check("the graph edits", ov && ov->editing());
    if (!ov) return;
    ov->settle();
    const auto dot = [&](int i) { return ov->mapToScene(ov->dotPoint(i)).toPoint(); };
    QTest::mouseClick(m.main, Qt::RightButton, Qt::NoModifier, dot(1));
    m.check("a right-click on a band's dot opens the same menu", m.open(menu) && m.row(menu, "band.copy"));
    QObject *own = ov->parentItem() ? ov->parentItem()->findChild<QObject *>(QStringLiteral("PeqEditor.bandMenu")) : nullptr;
    m.check("and Classic's stays closed", own && !own->property("visible").toBool());
    m.d.grab(m.main, QStringLiteral("modern-menu-dot.png"));
    m.check("and its rows act on that band", m.choose(menu, "band.invert") && m.d.waitFor(3000, [&] { return near(b(1).gain, -3); }) && near(b(0).gain, 4));
    ov->settle();
    QTest::mouseClick(m.main, Qt::RightButton, Qt::NoModifier, dot(1));
    m.check("Reset on a dot", m.open(menu) && m.choose(menu, "band.reset") && m.d.waitFor(3000, [&] { return b(1).filter_type == FILTER_FLAT; }));

    // Without the mouse: the menu key in a row's field
    if (QQuickItem *r = m.bandRow(0)) {
        QQuickItem *field = m.shown(QStringLiteral("*.freq.field"), r);
        if (field) field->forceActiveFocus(Qt::TabFocusReason);
        QTest::keyClick(m.main, Qt::Key_F10, Qt::ShiftModifier);
        m.check("Shift+F10 in a band's field opens its menu", m.open(menu) && m.enabled(menu, "band.reset"));
        m.close(menu);
    }
    // A crossover's band: bypass and reset only
    if (m.cc->crossoverSupported()) {
        m.cc->setCrossoverFamily(m.out, 0, 1);
        m.settle();
        QQuickItem *r = m.bandRow(0);
        const QVariant x = r ? r->mapToScene(QPointF(4, 4)) : QVariant();
        QMetaObject::invokeMethod(m.context, "band", Q_ARG(QVariant, m.out), Q_ARG(QVariant, 0), Q_ARG(QVariant, true),
                                  Q_ARG(QVariant, QVariant::fromValue(static_cast<QObject *>(m.main->contentItem()))), Q_ARG(QVariant, 400),
                                  Q_ARG(QVariant, 300), Q_ARG(QVariant, QVariant()));
        m.check("a crossover band's menu: bypass and reset", m.open(menu) && m.row(menu, "band.bypass") && m.row(menu, "band.reset")
                && !m.row(menu, "band.shape") && !m.row(menu, "band.paste") && !m.row(menu, "band.copy"));
        m.check("Reset turns it off", m.choose(menu, "band.reset") && m.d.waitFor(3000, [&] { return m.device()->state()->xover[m.out][0].filter_type == 0; }));
    }
}

void graphPart(Menus &m) {
    const char *menu = "ContextMenus.graph";
    auto *graph = Registrar::instance<GraphController>();
    m.cc->clearBands(m.out, false);
    m.ed->forget();
    m.settle();
    PeqOverlay *ov = m.overlay();
    if (!ov) return;
    const auto at = [&](double fx, double db) {
        return ov->mapToScene(QPointF(ov->width() * fx, (graph->dbTop() - db) / (graph->dbTop() - graph->dbBottom()) * ov->height())).toPoint();
    };
    const auto openAt = [&](double fx, double db) {
        QTest::mouseClick(m.main, Qt::RightButton, Qt::NoModifier, at(fx, db));
        return m.open(menu);
    };
    m.check("a right-click on the graph opens its menu", openAt(0.5, 5));
    m.check("the graph's menu is well formed", m.wellFormed(menu));
    m.check("no bands: nothing to select or delete", m.row(menu, "graph.selectAll") && !m.enabled(menu, "graph.selectAll") && !m.row(menu, "graph.delete"));
    m.d.grab(m.main, QStringLiteral("modern-menu-graph.png"));
    const double hz = graph->minFreq() * std::pow(graph->maxFreq() / graph->minFreq(), 0.5);
    m.check("Add a Band Here adds it there", m.choose(menu, "graph.addBand") && m.d.waitFor(3000, [&] {
        const FilterParams p = m.band(m.out, 0);
        return p.filter_type == FILTER_PEAKING && near(p.freq / hz, 1, 0.03) && near(p.gain, 5, 0.5);
    }));
    ov->settle();
    openAt(0.85, -12);
    m.check("Select All Bands selects", m.choose(menu, "graph.selectAll") && m.ed->selected() == QList<int>{ 0 });
    const bool phase = graph->property("showPhase").toBool();
    openAt(0.85, -12);
    m.check("the phase's row", m.text(menu, "graph.phase") == (phase ? QLatin1String("Hide the Phase") : QLatin1String("Show the Phase"))
            && m.choose(menu, "graph.phase") && graph->property("showPhase").toBool() != phase);
    graph->setProperty("showPhase", phase);
    openAt(0.85, -12);
    m.check("Delete the selection", m.choose(menu, "graph.delete") && m.d.waitFor(3000, [&] { return m.band(m.out, 0).filter_type == FILTER_FLAT; }));
    openAt(0.85, -12);
    m.check("Graph Settings goes to Settings › Graphing", m.choose(menu, "graph.settings") && m.shell->pageAction() == QLatin1String("edit.settings"));
    m.go("equaliser", "peq");
    ov = m.overlay();
    if (!ov) return;
    openAt(0.85, -12);
    m.check("Pop Out Graph pops it out", m.choose(menu, "window.popOutGraph") && m.d.waitFor(3000, [&] { return graph->property("poppedOut").toBool(); }));
    graph->closePopOut();
    m.d.waitFor(3000, [&] { return !graph->property("poppedOut").toBool(); });
    m.settle();
}

void slotPart(Menus &m) {
    const char *menu = "ContextMenus.slot";
    auto *presets = m.device()->presets();
    const auto activeSlot = [&] { return presets->property("activeSlot").toInt(); };
    if (m.sc->unsaved()) {
        m.sc->savePreset();
        m.d.waitFor(5000, [&] { return !m.sc->unsaved(); });
    }
    const int start = activeSlot(), other = start == 0 ? 1 : 0;
    m.settle();
    m.check("a right-click on the preset picker opens the active slot's menu", m.click(QStringLiteral("TopBar.preset"), Qt::RightButton) && m.open(menu));
    for (const char *id : { "slot.recall", "slot.save", "slot.rename", "slot.startup", "slot.copyTo" })
        m.d.check(QStringLiteral("lay3b: the active slot's menu has %1").arg(QLatin1String(id)), m.row(menu, id) != nullptr);
    m.check("the slot's menu is well formed", m.wellFormed(menu));
    m.check("the active slot is not recalled, nor copied on itself", !m.enabled(menu, "slot.recall")
            && !m.row(menu, qPrintable(QStringLiteral("slot.copyTo.%1").arg(start))) && m.row(menu, qPrintable(QStringLiteral("slot.copyTo.%1").arg(other))));
    m.d.grab(m.main, QStringLiteral("modern-menu-slot.png"));
    m.check("Rename asks for that slot's name", m.choose(menu, "slot.rename"));
    QObject *rename = m.d.object(QStringLiteral("GlobalControls.presetRename"));
    m.check("in the rename sheet", rename && m.d.waitFor(1500, [&] { return rename->property("opened").toBool(); }) && rename->property("slot").toInt() == start);
    if (rename) QMetaObject::invokeMethod(rename, "close");
    m.settle();

    // A slot in the list
    const auto openSlot = [&](int slot) {
        m.click(QStringLiteral("TopBar.preset"));
        QQuickItem *box = m.shown(QStringLiteral("TopBar.preset"));
        QObject *list = box ? box->property("popup").value<QObject *>() : nullptr;
        m.d.waitFor(1500, [&] { return list && list->property("opened").toBool(); });
        m.d.spin(80);
        QQuickItem *content = list ? list->property("contentItem").value<QQuickItem *>() : nullptr;
        QQuickItem *r = nullptr;
        if (content) QMetaObject::invokeMethod(content, "itemAtIndex", Q_RETURN_ARG(QQuickItem *, r), Q_ARG(int, slot));
        if (!r) return false;
        QTest::mouseClick(m.main, Qt::RightButton, Qt::NoModifier, m.point(r, 0.3));
        return m.open(menu);
    };
    m.check("a right-click on a slot in the list opens that slot's menu", openSlot(other) && m.enabled(menu, "slot.recall") && !m.row(menu, "slot.copyTo"));
    m.check("an empty one has no name and no startup", m.sc->presetOccupied(other) || (!m.enabled(menu, "slot.rename") && !m.enabled(menu, "slot.startup")));
    m.check("its save says what it does", m.text(menu, "slot.save").contains(m.sc->presetLabel(start)));
    m.d.grab(m.main, QStringLiteral("modern-menu-slot-list.png"));
    m.check("Copy Here keeps the active preset there", m.choose(menu, "slot.save") && m.d.waitFor(6000, [&] { return m.sc->presetOccupied(other) && !m.sc->presetBusy(); })
            && activeSlot() == start);
    m.settle();
    const int defaultSlot = presets->property("defaultSlot").toInt(), mode = presets->property("startupMode").toInt();
    m.check("Set as Startup Preset sets that one", openSlot(other) && m.choose(menu, "slot.startup")
            && m.d.waitFor(4000, [&] { return presets->property("defaultSlot").toInt() == other && presets->property("startupMode").toInt() == 0; }));
    if (mode == 0) {
        m.sc->setStartupPreset(defaultSlot);
        m.d.waitFor(4000, [&] { return presets->property("defaultSlot").toInt() == defaultSlot; });
    }
    m.check("Recall recalls it", openSlot(other) && m.choose(menu, "slot.recall") && m.d.waitFor(6000, [&] { return activeSlot() == other && !m.sc->presetBusy(); }));
    m.settle();
    m.sc->requestPresetSwitch(start);
    m.d.waitFor(6000, [&] { return activeSlot() == start && !m.sc->presetBusy(); });
    m.settle();
    m.check("Clear asks, naming the slot", openSlot(other) && m.text(menu, "slot.clear").contains(m.sc->presetLabel(other)) && m.choose(menu, "slot.clear"));
    QObject *clear = m.d.object(QStringLiteral("GlobalControls.clearQuestion"));
    m.check("its question shows", clear && m.d.waitFor(1500, [&] { return clear->property("opened").toBool(); }));
    if (clear) QMetaObject::invokeMethod(clear, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    m.check("and clears it", m.d.waitFor(6000, [&] { return !m.sc->presetOccupied(other) && !m.sc->presetBusy(); }));
    m.settle();
}

// Another device becomes active: what a menu is about stays behind
void leavingPart(Menus &m) {
    auto *registry = DeviceRegistry::instance();
    const QString first = registry->activeId();
    m.go("equaliser", "peq");
    m.sc->select(m.out);
    m.put(m.out, 0, FILTER_PEAKING, 500, 1, 6);
    m.settle();
    m.check("a band's menu is open", m.bandMenu(0));
    registry->openDemo();
    m.check("another device closes it", m.d.waitFor(15000, [&] { return registry->activeId() != first; })
            && m.d.waitFor(1500, [&] { return !m.menu("ContextMenus.band")->property("visible").toBool(); }));
    const QString second = registry->activeId();
    m.d.waitIdle();
    m.settle();
    registry->close(second);
    m.d.waitFor(8000, [&] { return registry->devices().size() == 1 && registry->activeId() == first; });
    m.d.waitIdle();
    m.settle();
    m.cc->clearBands(m.out, false);
    m.settle();

    // In Classic the components have their own menus
    m.check("back in Classic", switchTo(m, QStringLiteral("classic"), "ClassicShell"));
    m.click(QStringLiteral("Sidebar.channel.0"), Qt::RightButton, 0.3);
    m.d.spin(200);
    m.check("Classic's channel menu is its own", !m.menu("ContextMenus.channel")->property("visible").toBool());
    QTest::keyClick(m.main, Qt::Key_Escape);
    m.settle();
}

void menusSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    Menus m{ d, d.app().shell, Registrar::instance<LayoutController>(), Registrar::instance<ChannelController>(),
             Registrar::instance<SidebarController>(), Registrar::instance<PeqEditorController>(), d.mainWindow(),
             d.app().engine->singletonInstance<QObject *>("UConsole", "ContextMenus") };
    m.check("the controllers", m.shell && m.layout && m.cc && m.sc && m.ed && m.main && m.context && DeviceRegistry::instance());
    if (!m.layout || !m.cc || !m.sc || !m.ed || !m.main || !m.context) return;
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    m.check("on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    for (const QString &action : m.shell->shownWindows()) m.shell->closeWindow(action);
    for (QWindow *w : QGuiApplication::topLevelWindows())
        if (w->isVisible() && w != m.main && w->inherits("QQuickWindow")) w->close();
    m.main->requestActivate();
    d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == m.main; });
    const QSize was = m.main->size();
    const bool xoTab = m.cc->crossoverTabShown();
    m.cc->setCrossoverTabShown(false);
    m.main->resize(1280, 900);
    d.spin(200);
    m.check("Modern", switchTo(m, QStringLiteral("modern"), "ModernShell"));
    if (!m.shell->pageHost()) return;
    for (int o = 0; o < m.cc->numOutputs() && m.out < 0; ++o)
        if (m.cc->output(o).value(QStringLiteral("enabled")).toBool()) m.out = m.cc->outputChannel(o);
    m.check("an output is on", m.out >= 0);
    if (m.out < 0) return;

    buttonMenusPart(m);
    devicePart(m);
    channelPart(m);
    bandPart(m);
    graphPart(m);
    slotPart(m);
    leavingPart(m);

    m.cc->setCrossoverTabShown(xoTab);
    m.sc->select(-1);
    m.main->resize(was);
    d.spin(200);
}

const SmokeDriver::Section actionsSmoke("lay3b-actions", 896, &actionsSection);
const SmokeDriver::Section menusSmoke("lay3b-menus", 897, &menusSection);

} // namespace
