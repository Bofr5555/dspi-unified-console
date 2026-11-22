#ifndef SHELL_H
#define SHELL_H

#include <QHash>
#include <QObject>
#include <QPointer>
#include <QSize>
#include <QStringList>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <functional>

class QQmlEngine;
class QJSEngine;
#include <QWindow>

// The app shell's routing: every menu row and shortcut triggers an action id
// ("tools.matrix", "file.importFilters"; MenuTable.qml lists them). An id runs,
// in this order, the handler a controller added for it, or opens the tool
// window registered for it; an id nobody handles yet does nothing but log, so
// the full menu exists from the start and each work package fills its rows.
// -- WP LAY0 -- A tool window whose action has a PanelTable row is a Panel in
// a PanelWindow; while the loaded shell hosts panels on pages (pageHost, the
// Modern shell), trigger() shows the panel on its page instead.
//
// It also holds the registered banners, which BannerHost stacks, and the
// colour scheme the Theme follows: the system's, unless overridden.
class Shell : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QStringList banners READ banners NOTIFY bannersChanged)
    Q_PROPERTY(QString lastUnhandled READ lastUnhandled NOTIFY lastUnhandledChanged)
    Q_PROPERTY(bool darkScheme READ darkScheme NOTIFY darkSchemeChanged)
    // The system's fixed-pitch family (Theme.fontMono): Consolas on Windows,
    // the desktop's monospace on Linux
    Q_PROPERTY(QString monospaceFamily READ monospaceFamily CONSTANT)
    // -- WP LAY0 -- The loaded shell's page host, null when it has none
    // (Classic). It has `function showPanel(row)`, which shows PanelTable's
    // `row` on its page and returns true, or false to let a window open. The
    // shell sets it when it loads; it clears itself when the host goes.
    Q_PROPERTY(QObject *pageHost READ pageHost WRITE setPageHost NOTIFY pageHostChanged)
public:
    explicit Shell(QQmlEngine *engine, QObject *parent = nullptr);
    ~Shell() override;

    static Shell *instance();
    static Shell *create(QQmlEngine *, QJSEngine *);

    // Runs `handler` when `action` is triggered, while `receiver` exists. One
    // handler per id: a second one for the same id stops the app at startup.
    void addHandler(const QString &action, QObject *receiver, std::function<void()> handler);
    // Opens the QML type `type` of module UConsole as a window for `action`.
    void addWindow(const QString &action, const QString &type);
    void setBanners(const QStringList &types);
    // Destroys the tool windows it made; main() calls it before the QML engine goes.
    void releaseWindows();

    // Light or dark regardless of the system (Qt::ColorScheme::Unknown follows
    // the system again): DSPI_COLOR_SCHEME and the smoke driver's screenshots.
    void setColorSchemeOverride(Qt::ColorScheme scheme);
    // -- WP QPI3 -- the override in force, for putting it back after a screenshot
    Qt::ColorScheme colorSchemeOverride() const { return m_schemeOverride; }
    // Dark unless the scheme in force is light: dark is also the look when the
    // system says nothing
    bool darkScheme() const;
    static QString monospaceFamily();

    QStringList banners() const { return m_banners; }
    QString lastUnhandled() const { return m_lastUnhandled; }

    Q_INVOKABLE void trigger(const QString &action);
    // Whether something handles `action` (a menu row with `optional: true`
    // shows only then).
    Q_INVOKABLE bool handles(const QString &action) const;
    // The window registered for `action`, made and shown; null if none. With
    // a PanelTable row it is a PanelWindow, also while a page host is set
    // (the Modern "Open in window").
    Q_INVOKABLE QWindow *openWindow(const QString &action);
    // -- WP LAY0 -- PanelTable's row for `action`; empty when its tool is no panel
    Q_INVOKABLE QVariantMap panel(const QString &action) const;
    // Every PanelTable row, in order
    QVariantList panels() const;
    QObject *pageHost() const { return m_pageHost; }
    void setPageHost(QObject *host);
    Q_INVOKABLE void openUrl(const QString &url);
    Q_INVOKABLE void quit();

    // -- WP QP15 -- Asked before quit() quits (UnsavedGuard: the unsaved-changes
    // question), while `receiver` exists; `guard` calls `proceed` to quit
    void setQuitGuard(QObject *receiver, std::function<void(std::function<void()> proceed)> guard);

    // -- WP QTSENDER -- Asked by trigger() before it runs `action`, while
    // `receiver` exists; false refuses it (DeviceInfoController: nothing that
    // needs a DSP runs on the alink sender). A window made by openWindow()
    // gets its action id in its `actionId` property, when it has one.
    void setActionGate(QObject *receiver, std::function<bool(const QString &action)> gate);
    // Whether the gate lets `action` run now
    Q_INVOKABLE bool allows(const QString &action) const;

    // -- WP QTPOP -- The most a tool window grows to fit its content: the main
    // window's size (while it shows) and the available size of `window`'s
    // screen, less kFitMargin on each side; device-independent pixels.
    static constexpr int kFitMargin = 48;
    Q_INVOKABLE QSize fitCap(const QWindow *window) const;
    // Sizes `window` to width x height: a side that grows stops at fitCap()
    // (or where it is, when that is already more); one that shrinks takes the
    // size asked, down to the window's minimum. Moves it up or left when it
    // would leave its screen. Returns the size it gave it.
    Q_INVOKABLE QSize fitWindow(QWindow *window, int width, int height) const;

signals:
    void triggered(const QString &action);
    void bannersChanged();
    void lastUnhandledChanged();
    void darkSchemeChanged();
    void pageHostChanged();   // -- WP LAY0 --

private:
    struct Handler {
        QPointer<QObject> receiver;
        std::function<void()> run;
    };
    QQmlEngine *m_engine;
    QHash<QString, Handler> m_handlers;
    QHash<QString, QString> m_windowTypes;
    QHash<QString, QPointer<QWindow>> m_windows;
    QStringList m_banners;
    QString m_lastUnhandled;
    Qt::ColorScheme m_schemeOverride = Qt::ColorScheme::Unknown;
    // -- WP QP15 --
    QPointer<QObject> m_quitGuardReceiver;
    std::function<void(std::function<void()>)> m_quitGuard;
    // -- WP QTSENDER --
    QPointer<QObject> m_gateReceiver;
    std::function<bool(const QString &)> m_gate;
    // -- WP LAY0 --
    QPointer<QObject> m_pageHost;
    QMetaObject::Connection m_pageHostGone;
};

#endif // SHELL_H
