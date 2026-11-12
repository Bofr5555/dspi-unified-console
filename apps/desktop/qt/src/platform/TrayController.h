#ifndef TRAYCONTROLLER_H
#define TRAYCONTROLLER_H

#include <QList>
#include <QObject>
#include <QPointer>
#include <QTimer>
#include <QWindow>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "Registrar.h"

class DeviceModel;
class HubController;
class QAction;
class QMenu;
class QQmlEngine;
class QJSEngine;
class QQuickWindow;
class QSystemTrayIcon;
class Shell;
class SidebarController;

// -- WP QP14 -- The tray (behaviour.md §12.1, the Mac's MenuBarMode): with
// "Show in the tray" on and a tray on the desktop, closing the main window hides
// it while the app, the hub and the polls carry on; the icon's menu shows what
// the hub is doing and offers Open, sharing, pairing, the volume and Quit.
// "Start in the tray only" starts hidden. With no tray the window always shows
// and closing it quits, whatever the settings say. QML sees it as the singleton
// `TrayController`; the switches are on the Networking page.
class TrayController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // tray/showInTray, tray/startMinimised (behaviour.md §21.2)
    Q_PROPERTY(bool showInTray READ showInTray WRITE setShowInTray NOTIFY changed)
    Q_PROPERTY(bool startMinimised READ startMinimised WRITE setStartMinimised NOTIFY changed)
    // The desktop has a tray now
    Q_PROPERTY(bool trayAvailable READ trayAvailable NOTIFY changed)
    Q_PROPERTY(bool iconShown READ iconShown NOTIFY changed)
    Q_PROPERTY(bool windowHidden READ windowHidden NOTIFY changed)

public:
    TrayController(const AppContext &ctx, QObject *parent);
    ~TrayController() override;
    static TrayController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<TrayController>(); }

    bool showInTray() const { return m_showInTray; }
    void setShowInTray(bool on);
    bool startMinimised() const { return m_startMinimised; }
    void setStartMinimised(bool on);
    bool trayAvailable() const { return m_available; }
    bool iconShown() const { return m_icon != nullptr; }
    bool windowHidden() const { return m_hidden; }

    // Hides the main window (and the tool windows showing) to the tray, turning
    // the tray on as the Mac's does; a plain minimise when there is no tray
    Q_INVOKABLE void minimise();
    // The windows back as they were, in front
    Q_INVOKABLE void restore();
    // Quits through the menu's own Quit (file.quit), so its guards apply
    Q_INVOKABLE void quit();

    // For the smoke driver: the icon and its menu (null while not shown), a stand-in
    // for the desktop's answer, and asking it again now
    QSystemTrayIcon *icon() const { return m_icon; }
    QMenu *contextMenu() const { return m_menu; }
    void setAvailabilityProbe(std::function<bool()> probe);
    void recheckTray();
    bool rechecking() const { return m_recheck.isActive(); }
    // The main window, as the engine makes it (and the smoke driver's start in
    // the tray, made again): closing it may hide it, and with both settings on
    // and a tray it starts hidden
    void adoptWindow(QObject *root);

signals:
    void changed();

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void update();
    void buildMenu();
    void dropIcon();
    void refreshMenu();
    void refreshIcon();
    void stepVolume(double db);
    void hideWindows();
    HubController *hub();
    SidebarController *sidebar();

    Shell *m_shell;
    DeviceModel *m_device;
    QPointer<QQuickWindow> m_main;
    QPointer<HubController> m_hub;
    QPointer<SidebarController> m_sidebar;
    std::function<bool()> m_probe;
    bool m_showInTray = false;
    bool m_startMinimised = false;
    bool m_available = false;
    bool m_hidden = false;
    bool m_iconSharing = false;
    QWindow::Visibility m_mainVisibility = QWindow::Windowed;
    QList<QPointer<QWindow>> m_hiddenTools;
    QSystemTrayIcon *m_icon = nullptr;
    QMenu *m_menu = nullptr;
    QTimer m_recheck;   // while the tray is wanted and the desktop has none
};

#endif // TRAYCONTROLLER_H
