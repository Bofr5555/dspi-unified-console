#ifndef SHELL_H
#define SHELL_H

#include <QHash>
#include <QObject>
#include <QPointer>
#include <QStringList>
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
    // The window registered for `action`, made and shown; null if none.
    Q_INVOKABLE QWindow *openWindow(const QString &action);
    Q_INVOKABLE void openUrl(const QString &url);
    Q_INVOKABLE void quit();

signals:
    void triggered(const QString &action);
    void bannersChanged();
    void lastUnhandledChanged();
    void darkSchemeChanged();

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
};

#endif // SHELL_H
