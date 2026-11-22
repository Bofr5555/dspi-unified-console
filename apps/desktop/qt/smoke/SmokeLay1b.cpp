// -- WP LAY1b -- The measure, analyse, library, system, devices and settings
// windows as panels, on the demo device: each of the ten opens by its action in
// a PanelWindow named as before, holding its panel with the old item names,
// sized and titled by its PanelTable row, and knows when it shows; what the
// windows used of their window goes through the host (a panel asks to close,
// Measure hears its window close, Spectrum hears it minimised, Firmware's
// least height grows); and a page host shows every one of them on a page,
// where the Settings search finds pages as it does in the window.
#include "SmokeDriver.h"

#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"

#include <QFont>
#include <QJSValue>
#include <QPointer>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QWindow>

#include <memory>

namespace {

struct Converted {
    const char *action;
    const char *type;    // the window's name (Registrar::Window)
    const char *panel;   // the panel's type and name
    const char *item;    // one of its items, by its old name
};

const Converted kConverted[] = {
    { "tools.measure", "MeasureWindow", "MeasurePanel", "Measure.tabs" },
    { "tools.signalGenerator", "SiggenWindow", "SiggenPanel", "SiggenWindow.state" },
    { "tools.spectrum", "SpectrumWindow", "SpectrumPanel", "SpectrumWindow.status" },
    { "device.snapshots", "SnapshotsWindow", "SnapshotsPanel", "SnapshotsWindow.list" },
    { "edit.history", "HistoryWindow", "HistoryPanel", "HistoryWindow.list" },
    { "autoeq.browse", "AutoEqWindow", "AutoEqPanel", "AutoEqWindow.search" },
    { "device.events", "EventsWindow", "EventsPanel", "Events.list" },
    { "device.firmwareUpdate", "FirmwareWindow", "FirmwarePanel", "FirmwareWindow.summary" },
    { "help.whatsNew", "WhatsNewWindow", "WhatsNewPanel", "WhatsNew.list" },
    { "edit.settings", "SettingsWindow", "SettingsPanel", "Settings.search" },
};

QQuickWindow *openByAction(SmokeDriver &d, Shell *shell, const QString &action) {
    shell->trigger(action);
    QWindow *window = nullptr;
    d.waitFor(3000, [&] {
        window = shell->openWindow(action);
        return window && window->isVisible();
    });
    return qobject_cast<QQuickWindow *>(window);
}

QQuickItem *contentOf(QQuickWindow *w) { return w ? w->property("content").value<QQuickItem *>() : nullptr; }

QStringList listedPages(QObject *settingsPanel) {
    QStringList ids;
    const QVariant rows = settingsPanel ? settingsPanel->property("rows") : QVariant();
    const QVariantList list = rows.canConvert<QJSValue>() ? rows.value<QJSValue>().toVariant().toList() : rows.toList();
    for (const QVariant &row : list) {
        const QString id = row.toMap().value(QStringLiteral("id")).toString();
        if (!id.isEmpty()) ids << id;
    }
    return ids;
}

// The Settings search in the host whose panel is `panel`: "phase" leaves
// Graphing alone, and Return (openFirst) opens it there
void settingsSearch(SmokeDriver &d, SettingsController *settings, QQuickItem *panel, const QString &host) {
    QQuickItem *search = panel ? d.item(QStringLiteral("Settings.search"), panel) : nullptr;
    QQuickItem *page = panel ? d.item(QStringLiteral("Settings.page"), panel) : nullptr;
    d.check(QStringLiteral("lay1b: %1 has the Settings search and page").arg(host), search && page);
    if (!search || !page) return;
    const QString before = settings->currentPage();
    settings->open(QStringLiteral("advanced"));
    search->setProperty("text", QStringLiteral("phase"));
    d.spin(150);
    d.check(QStringLiteral("lay1b: the Settings search finds Graphing for \"phase\" in %1").arg(host),
            listedPages(panel) == QStringList{ QStringLiteral("graphing") });
    QMetaObject::invokeMethod(search, "openFirst");
    d.spin(200);
    d.check(QStringLiteral("lay1b: Return opens the page found in %1").arg(host),
            settings->currentPage() == QLatin1String("graphing") && page->property("item").value<QQuickItem *>() != nullptr);
    search->setProperty("text", QString());
    d.spin(100);
    QQuickItem *noMatch = d.item(QStringLiteral("Settings.noMatch"), panel);
    d.check(QStringLiteral("lay1b: an empty search lists every page again in %1").arg(host),
            listedPages(panel).size() > 5 && noMatch && !noMatch->isVisible());
    if (!before.isEmpty()) settings->open(before);
}

void windowsPart(SmokeDriver &d, Shell *shell, SettingsController *settings) {
    for (const Converted &c : kConverted) {
        const QString action = QString::fromLatin1(c.action);
        const QVariantMap row = shell->panel(action);
        d.check(QStringLiteral("lay1b: %1 has its PanelTable row").arg(action),
                row.value(QStringLiteral("panel")).toString() == QLatin1String(c.panel));
        QQuickWindow *quick = openByAction(d, shell, action);
        QQuickItem *content = contentOf(quick);
        d.check(QStringLiteral("lay1b: %1 opens in a PanelWindow by its action").arg(action),
                quick && quick->isVisible() && content
                    && QString::fromLatin1(quick->metaObject()->className()).startsWith(QLatin1String("PanelWindow")));
        if (!quick || !content) continue;
        d.check(QStringLiteral("lay1b: %1's window keeps its name, and holds %2 with its old item names").arg(action, QLatin1String(c.panel)),
                quick->objectName() == QLatin1String(c.type) && content->objectName() == QLatin1String(c.panel)
                    && d.item(QLatin1String(c.item), quick->contentItem()) != nullptr);
        d.check(QStringLiteral("lay1b: %1's window takes the row's title, sizes and place").arg(action),
                quick->title() == row.value(QStringLiteral("title")).toString()
                    && quick->minimumWidth() == row.value(QStringLiteral("minWidth")).toInt()
                    && quick->minimumHeight() == qMax(row.value(QStringLiteral("minHeight")).toInt(), content->property("minimumHeight").toInt())
                    && quick->property("defaultWidth").toInt() == row.value(QStringLiteral("width")).toInt()
                    && quick->property("defaultHeight").toInt() == row.value(QStringLiteral("height")).toInt()
                    && quick->property("windowId").toString() == row.value(QStringLiteral("windowId")).toString());
        d.check(QStringLiteral("lay1b: %1 knows it shows, and its action").arg(QLatin1String(c.panel)),
                content->property("shown").toBool() && content->property("actionId").toString() == action
                    && !content->property("locked").toBool() && !content->property("minimized").toBool());

        if (action == QLatin1String("edit.settings")) {
            d.check(QStringLiteral("lay1b: the Settings window keeps the body font"),
                    quick->property("font").value<QFont>() == content->property("windowFont").value<QFont>());
            settingsSearch(d, settings, content, QStringLiteral("the Settings window"));
            // The panel's close button asks its host, and the window closes
            QMetaObject::invokeMethod(content, "close");
            d.check(QStringLiteral("lay1b: a panel's close() closes its window"),
                    d.waitFor(2000, [&] { return !quick->isVisible(); }) && !content->property("shown").toBool());
            continue;
        }
        if (action == QLatin1String("help.whatsNew")) {
            if (QQuickItem *done = d.item(QStringLiteral("WhatsNew.done"), quick->contentItem())) QMetaObject::invokeMethod(done, "click");
            d.check(QStringLiteral("lay1b: What's New's Done closes its window"), d.waitFor(2000, [&] { return !quick->isVisible(); }));
            continue;
        }
        if (action == QLatin1String("tools.spectrum")) {
            quick->setWindowState(Qt::WindowMinimized);
            d.check(QStringLiteral("lay1b: Spectrum hears its window minimised and stops rendering"),
                    d.waitFor(2000, [&] { return content->property("minimized").toBool() && !content->property("rendering").toBool(); }));
            quick->setWindowState(Qt::WindowNoState);
            quick->show();
            d.check(QStringLiteral("lay1b: and renders again once restored"),
                    d.waitFor(2000, [&] { return !content->property("minimized").toBool() && content->property("rendering").toBool(); }));
        }
        // The window's close reaches the panel (Measure stops what it started on it)
        QQmlComponent listen(d.app().engine);
        listen.setData("import QtQuick\n"
                       "Connections {\n"
                       "    property int heard: 0\n"
                       "    function onClosing() { heard++ }\n"
                       "}\n",
                       QUrl(QStringLiteral("qrc:/smoke/lay1b-closing.qml")));
        std::unique_ptr<QObject> heard(listen.createWithInitialProperties({ { QStringLiteral("target"), QVariant::fromValue(content) } }));
        quick->close();
        d.waitFor(2000, [&] { return !quick->isVisible(); });
        d.check(QStringLiteral("lay1b: %1 knows it no longer shows, and heard its window close").arg(QLatin1String(c.panel)),
                !content->property("shown").toBool() && heard && heard->property("heard").toInt() == 1);
    }
}

// A page host as Modern's will be (Shell.pageHost, showPanel(row)): it makes
// the row's panel on a page of its own and shows it there
void pagesPart(SmokeDriver &d, Shell *shell, SettingsController *settings) {
    QQmlComponent probe(d.app().engine);
    probe.setData("import QtQuick\n"
                  "import UConsole\n"
                  "Item {\n"
                  "    property Item current: null\n"
                  "    property string action\n"
                  "    function showPanel(row) {\n"
                  "        if (current) current.destroy()\n"
                  "        const made = Qt.createComponent(\"UConsole\", row.panel)\n"
                  "        current = made.createObject(this, { width: width, height: height, actionId: row.action, shown: true })\n"
                  "        action = row.action\n"
                  "        return current !== null\n"
                  "    }\n"
                  "}\n",
                  QUrl(QStringLiteral("qrc:/smoke/lay1b-pagehost.qml")));
    QPointer<QObject> host = probe.create();
    auto *hostItem = qobject_cast<QQuickItem *>(host.data());
    d.check(QStringLiteral("lay1b: the page host probe loads"), hostItem != nullptr && d.mainWindow());
    if (!hostItem || !d.mainWindow()) {
        std::printf("smoke: lay1b: %s\n", qPrintable(probe.errorString()));
        delete host.data();
        return;
    }
    hostItem->setParentItem(d.mainWindow()->contentItem());
    hostItem->setSize(QSizeF(900, 640));
    shell->setPageHost(host);
    for (const Converted &c : kConverted) {
        const QString action = QString::fromLatin1(c.action);
        QWindow *window = shell->openWindow(action);
        if (window) window->close();
        d.waitFor(2000, [&] { return window && !window->isVisible(); });
        shell->trigger(action);
        d.spin(150);
        auto *page = host->property("current").value<QQuickItem *>();
        d.check(QStringLiteral("lay1b: with a page host, %1 shows %2 on its page with its old item names, and no window").arg(action, QLatin1String(c.panel)),
                host->property("action").toString() == action && page && page->objectName() == QLatin1String(c.panel)
                    && page->property("shown").toBool() && d.item(QLatin1String(c.item), page) != nullptr && window && !window->isVisible());
        if (page && action == QLatin1String("edit.settings")) settingsSearch(d, settings, page, QStringLiteral("a page"));
    }
    shell->setPageHost(nullptr);
    delete host.data();
    d.spin(100);
}

void lay1bSection(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    auto *settings = Registrar::instance<SettingsController>();
    d.check(QStringLiteral("lay1b: the shell and the settings controller"), shell && settings);
    if (!shell || !settings) return;
    windowsPart(d, shell, settings);
    pagesPart(d, shell, settings);
}

const SmokeDriver::Section lay1bSmoke("lay1b-panels", 885, &lay1bSection);

} // namespace
