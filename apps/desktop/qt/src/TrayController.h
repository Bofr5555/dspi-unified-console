#ifndef TRAYCONTROLLER_H
#define TRAYCONTROLLER_H

#include <QIcon>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QTimer>
#include <QWindow>
#include <QWindowList>
#include <functional>
#include <memory>

class DSPiBridge;
class QAction;
class QMenu;
class QSystemTrayIcon;

// What the tray menu needs from the hub. QD14's controller implements this and
// calls TrayController::setHub; until one does, Share on This Network and Allow a
// New Client stay hidden. All of these are called on the UI thread, from a menu:
// they must not block.
class TrayHub : public QObject
{
    Q_OBJECT
public:
    struct State {
        bool running = false;       // the hub is listening
        bool sharing = false;       // the user's switch, which a failed start leaves on
        QString address;            // what clients dial; shown while running
        QString pairingCode;        // the active code, empty when none
        bool pinAccess = false;     // access is by PIN, so a new client can be allowed
        int clients = 0;
    };
    using QObject::QObject;
    virtual State trayState() const = 0;
    virtual void setSharing(bool on) = 0;
    virtual void allowNewClient() = 0;
    virtual void cancelPairing() = 0;
signals:
    void trayStateChanged();
};

// The tray icon and the window's way into it (MenuBarMode.swift). QML sees it as
// `tray`.
//
// With showInTray on and a tray there, closing the main window hides it, with any
// tool windows open, and asks nothing since nothing is lost. The bridge, a Link
// session and the hub carry on. A real quit (the menu's Quit, the Quit shortcut, the
// session ending) goes through main.qml's onClosing as ever, so unsaved changes are
// still asked about.
//
// Without a tray the window is never hidden, as nothing could bring it back: close
// quits, start minimised is ignored, and a tray lost while hidden restores the
// window. Availability is rechecked every few seconds while showInTray is on, and
// never while it is off.
class TrayController : public QObject
{
    Q_OBJECT
    // Settings, for QD14's "Running in the background" section
    Q_PROPERTY(bool showInTray READ showInTray WRITE setShowInTray NOTIFY settingsChanged)
    Q_PROPERTY(bool startMinimised READ startMinimised WRITE setStartMinimised NOTIFY settingsChanged)
    // Whether this desktop has a tray right now; say so next to the switch if not
    Q_PROPERTY(bool trayAvailable READ trayAvailable NOTIFY stateChanged)
    Q_PROPERTY(bool iconShown READ iconShown NOTIFY stateChanged)
    Q_PROPERTY(bool windowHidden READ windowHidden NOTIFY stateChanged)

public:
    TrayController(DSPiBridge *bridge, QObject *parent);
    ~TrayController() override;

    // main.cpp, around loading main.qml: whether the window should start hidden,
    // which needs both settings and a tray now; and the window, once it exists.
    static bool startsHidden();
    static void attachMainWindow(QWindow *window);

    // QD14: the hub behind the two hub items and the status line. Either order
    // against this controller's construction; nullptr removes it.
    static void setHub(TrayHub *hub);

    bool showInTray() const { return m_show; }
    void setShowInTray(bool on);
    bool startMinimised() const { return m_startMinimised; }
    void setStartMinimised(bool on);
    bool trayAvailable() const;
    bool iconShown() const { return m_icon != nullptr; }
    bool windowHidden() const { return m_hidden; }

    // Hide to the tray, turning it on as the Mac's does. Refused with no tray.
    Q_INVOKABLE void minimise();
    // Show the window and bring it forward, hidden or not
    Q_INVOKABLE void restore();
    // What the menu's Quit does: as closing the window with no tray. A prompt for
    // unsaved changes brings the window back; otherwise it stays hidden.
    Q_INVOKABLE void quit();

    // For the smoke driver: the menu, the icon, and a tray to fake. Offscreen has none.
    QMenu *contextMenu() const { return m_menu.get(); }
    QSystemTrayIcon *icon() const { return m_icon; }
    void setAvailabilityProbe(std::function<bool()> probe);
    void recheckTray();
    bool rechecking() const { return m_poll.isActive(); }

signals:
    void settingsChanged();
    void stateChanged();

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void reconcile();
    void makeIcon();
    void removeIcon();
    void refreshMenu();
    QString statusLine(const TrayHub::State *hub) const;

    DSPiBridge *m_bridge;
    std::function<bool()> m_probe;
    QTimer m_poll;
    bool m_show = false;
    bool m_startMinimised = false;
    bool m_available = false;

    QPointer<QWindow> m_window;
    bool m_hidden = false;
    QWindow::Visibility m_shownAs = QWindow::AutomaticVisibility;
    QList<QPointer<QWindow>> m_hiddenTools;

    bool m_realClose = false;   // this close is a quit, not a hide
    bool m_quitting = false;    // the application is on its way out, for good

    QSystemTrayIcon *m_icon = nullptr;
    std::unique_ptr<QMenu> m_menu;
    QAction *m_status = nullptr;
    QAction *m_address = nullptr;
    QAction *m_pairingCode = nullptr;
    QAction *m_open = nullptr;
    QAction *m_share = nullptr;
    QAction *m_pair = nullptr;
    bool m_iconSharing = false;
};

#endif // TRAYCONTROLLER_H
