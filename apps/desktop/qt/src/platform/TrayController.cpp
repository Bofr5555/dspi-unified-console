#include "TrayController.h"

#include "AppIdentity.h"
#include "DeviceModel.h"
#include "HubController.h"
#include "Shell.h"
#include "SidebarController.h"

#include <QAction>
#include <QApplication>
#include <QCloseEvent>
#include <QFile>
#include <QGuiApplication>
#include <QIcon>
#include <QMenu>
#include <QPainter>
#include <QPalette>
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <QSettings>
#include <QSvgRenderer>
#include <QSystemTrayIcon>

static const Registrar::Controller<TrayController> trayRegistered;

namespace {

const char *const kShowKey = "tray/showInTray";
const char *const kStartKey = "tray/startMinimised";
constexpr double kVolumeStep = 3.0;     // dB per Volume Up / Down
constexpr double kVolumeFloor = -60.0;  // where Volume Up starts from silence

// The menu bar glyph of the Mac (waveform.circle, filled while sharing), from
// the Lucide waveform in the system's text colour
QIcon trayIcon(bool sharing) {
    QFile f(QStringLiteral(":/qt/qml/UConsole/icons/lucide/audio-waveform.svg"));
    const QByteArray svg = f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray();
    const QPalette palette = QGuiApplication::palette();
    const QColor ink = palette.color(QPalette::WindowText);
    const QColor paper = palette.color(QPalette::Window);
    QIcon icon;
    for (const int size : { 16, 22, 24, 32, 48, 64 }) {
        QPixmap pixmap(size, size);
        pixmap.fill(Qt::transparent);
        QPainter p(&pixmap);
        p.setRenderHint(QPainter::Antialiasing);
        const qreal pen = qMax(1.0, size / 14.0);
        const QRectF disc(pen / 2, pen / 2, size - pen, size - pen);
        QByteArray glyph = svg;
        if (sharing) {
            p.setPen(Qt::NoPen);
            p.setBrush(ink);
            p.drawEllipse(disc);
            glyph.replace("currentColor", paper.name(QColor::HexRgb).toLatin1());
        } else {
            p.setPen(QPen(ink, pen));
            p.setBrush(Qt::NoBrush);
            p.drawEllipse(disc);
            glyph.replace("currentColor", ink.name(QColor::HexRgb).toLatin1());
        }
        QSvgRenderer renderer(glyph);
        const qreal inset = size * 0.24;
        renderer.render(&p, QRectF(inset, inset, size - 2 * inset, size - 2 * inset));
        p.end();
        icon.addPixmap(pixmap);
    }
    return icon;
}

QAction *addItem(QMenu *menu, const char *name) {
    QAction *a = menu->addAction(QString());
    a->setObjectName(QLatin1String(name));
    return a;
}

QAction *child(QMenu *menu, const char *name) {
    return menu ? menu->findChild<QAction *>(QLatin1String(name)) : nullptr;
}

} // namespace

TrayController::TrayController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_shell(ctx.shell), m_device(ctx.device) {
    QSettings s;
    m_showInTray = s.value(QLatin1String(kShowKey), false).toBool();
    m_startMinimised = s.value(QLatin1String(kStartKey), false).toBool();

    m_recheck.setInterval(5000);
    connect(&m_recheck, &QTimer::timeout, this, &TrayController::recheckTray);

    // The main window, as the engine makes it: closing it may hide it, and
    // starting in the tray hides it before it is used
    if (auto *engine = qobject_cast<QQmlApplicationEngine *>(ctx.engine)) {
        connect(engine, &QQmlApplicationEngine::objectCreated, this, [this](QObject *root, const QUrl &) {
            if (!m_main) adoptWindow(root);
        });
    }
    ctx.shell->addHandler(QStringLiteral("window.minimiseToTray"), this, [this]() { minimise(); });
    connect(m_device, &DeviceModel::snapshotApplied, this, &TrayController::refreshMenu);
    update();
}

TrayController::~TrayController() { dropIcon(); }

void TrayController::adoptWindow(QObject *root) {
    m_main = qobject_cast<QQuickWindow *>(root);
    if (!m_main) return;
    m_main->installEventFilter(this);
    if (m_showInTray && m_startMinimised && m_icon) hideWindows();
}

HubController *TrayController::hub() {
    if (!m_hub) {
        m_hub = Registrar::instance<HubController>();
        if (m_hub) {
            connect(m_hub, &HubController::stateChanged, this, &TrayController::refreshMenu);
            connect(m_hub, &HubController::settingsChanged, this, &TrayController::refreshMenu);
            connect(m_hub, &HubController::pairingChanged, this, &TrayController::refreshMenu);
            connect(m_hub, &HubController::sharingChanged, this, &TrayController::refreshMenu);
        }
    }
    return m_hub;
}

SidebarController *TrayController::sidebar() {
    if (!m_sidebar) {
        m_sidebar = Registrar::instance<SidebarController>();
        if (m_sidebar) {
            connect(m_sidebar, &SidebarController::deviceChanged, this, &TrayController::refreshMenu);
            connect(m_sidebar, &SidebarController::volumeModeChanged, this, &TrayController::refreshMenu);
        }
    }
    return m_sidebar;
}

// ---- Settings and the desktop's tray -----------------------------------------------

void TrayController::setShowInTray(bool on) {
    if (on == m_showInTray) return;
    m_showInTray = on;
    QSettings().setValue(QLatin1String(kShowKey), on);
    update();   // off while hidden brings the window back: there would be no way back
}

void TrayController::setStartMinimised(bool on) {
    if (on == m_startMinimised) return;
    m_startMinimised = on;
    QSettings().setValue(QLatin1String(kStartKey), on);
    emit changed();
}

void TrayController::setAvailabilityProbe(std::function<bool()> probe) {
    m_probe = std::move(probe);
    update();
}

void TrayController::recheckTray() { update(); }

void TrayController::update() {
    m_available = m_probe ? m_probe() : QSystemTrayIcon::isSystemTrayAvailable();
    const bool want = m_showInTray && m_available;
    if (want && !m_icon) buildMenu();
    else if (!want && m_icon) dropIcon();
    // With an icon the app outlives its windows; without one, closing the last quits
    QGuiApplication::setQuitOnLastWindowClosed(!want);
    if (!want && m_hidden) restore();
    if (m_showInTray && !m_available) {
        if (!m_recheck.isActive()) m_recheck.start();
    } else {
        m_recheck.stop();
    }
    emit changed();
}

// ---- The icon and its menu ------------------------------------------------------

void TrayController::buildMenu() {
    m_menu = new QMenu;
    // The Mac's menu bar menu (MenuBarMode.swift), in its order, and the volume
    addItem(m_menu, "Tray.status")->setEnabled(false);
    addItem(m_menu, "Tray.address")->setEnabled(false);
    addItem(m_menu, "Tray.pairingCode")->setEnabled(false);
    m_menu->addSeparator();
    connect(addItem(m_menu, "Tray.open"), &QAction::triggered, this, &TrayController::restore);
    QAction *share = addItem(m_menu, "Tray.share");
    share->setCheckable(true);
    connect(share, &QAction::triggered, this, [this](bool on) {
        if (HubController *h = hub()) h->setSharing(on);
        refreshMenu();   // the tick follows the hub, not the click
    });
    connect(addItem(m_menu, "Tray.pair"), &QAction::triggered, this, [this]() {
        HubController *h = hub();
        if (!h) return;
        if (h->pairingCode().isEmpty()) h->allowNewClient();
        else h->cancelPairing();
    });
    m_menu->addSeparator();
    QMenu *volume = m_menu->addMenu(QString());
    volume->setObjectName(QStringLiteral("Tray.volumeMenu"));
    volume->menuAction()->setObjectName(QStringLiteral("Tray.volume"));
    addItem(volume, "Tray.volumeLevel")->setEnabled(false);
    connect(addItem(volume, "Tray.volumeUp"), &QAction::triggered, this, [this]() { stepVolume(kVolumeStep); });
    connect(addItem(volume, "Tray.volumeDown"), &QAction::triggered, this, [this]() { stepVolume(-kVolumeStep); });
    m_menu->addSeparator();
    connect(addItem(m_menu, "Tray.quit"), &QAction::triggered, this, &TrayController::quit);

    m_icon = new QSystemTrayIcon(this);
    m_icon->setContextMenu(m_menu);
    m_icon->setToolTip(AppIdentity::displayName());
    connect(m_icon, &QSystemTrayIcon::activated, this, [this](QSystemTrayIcon::ActivationReason reason) {
        // A click or a double click shows the window; the context click opens the menu
        if (reason == QSystemTrayIcon::Trigger || reason == QSystemTrayIcon::DoubleClick) restore();
    });
    m_iconSharing = false;
    m_icon->setIcon(trayIcon(false));
    refreshMenu();
    m_icon->show();
}

void TrayController::dropIcon() {
    if (m_icon) {
        m_icon->hide();
        delete m_icon;
        m_icon = nullptr;
    }
    delete m_menu;
    m_menu = nullptr;
}

void TrayController::refreshIcon() {
    if (!m_icon) return;
    const bool sharing = m_hub && m_hub->running();
    if (sharing == m_iconSharing) return;
    m_iconSharing = sharing;
    m_icon->setIcon(trayIcon(sharing));
}

void TrayController::refreshMenu() {
    if (!m_menu) return;
    HubController *h = hub();
    SidebarController *bar = sidebar();
    const QString name = AppIdentity::displayName();
    const bool running = h && h->running();

    const QString device = m_device->connected() ? tr("DSPi connected") : tr("No DSPi connected");
    QString status = device;
    if (h && !running) status = tr("%1, not sharing").arg(device);
    else if (h) status = h->sessions() == 1 ? tr("%1, sharing, 1 client").arg(device)
                                            : tr("%1, sharing, %2 clients").arg(device).arg(h->sessions());
    child(m_menu, "Tray.status")->setText(status);
    QAction *address = child(m_menu, "Tray.address");
    address->setText(running ? h->address() : QString());
    address->setVisible(running && !h->address().isEmpty());
    QAction *code = child(m_menu, "Tray.pairingCode");
    const QString pin = h ? h->pairingCode() : QString();
    code->setText(tr("Pairing code %1").arg(pin));
    code->setVisible(!pin.isEmpty());

    child(m_menu, "Tray.open")->setText(m_hidden ? tr("Open %1").arg(name) : tr("Show %1").arg(name));
    QAction *share = child(m_menu, "Tray.share");
    share->setText(tr("Share on This Network"));
    share->setVisible(h != nullptr);
    share->setChecked(h && h->sharing() && h->lastError().isEmpty());
    QAction *pair = child(m_menu, "Tray.pair");
    pair->setText(pin.isEmpty() ? tr("Allow a New Client…") : tr("Cancel Pairing"));
    pair->setVisible(h && h->access() == QLatin1String("pin"));
    pair->setEnabled(running);

    // The volume the sidebar shows (User or Master), while a device is open
    QAction *volume = child(m_menu, "Tray.volume");
    volume->setText(tr("Volume"));
    volume->setVisible(bar && m_device->connected());
    if (bar) {
        const bool master = bar->volumeMode() == QLatin1String("master");
        child(m_menu, "Tray.volumeLevel")
            ->setText(master ? tr("Master Volume: %1").arg(SidebarController::formatMasterDb(bar->masterVolumeDb()))
                             : tr("User Volume: %1").arg(SidebarController::formatUserDb(bar->userVolumeDb())));
    }
    child(m_menu, "Tray.volumeUp")->setText(tr("Volume Up"));
    child(m_menu, "Tray.volumeDown")->setText(tr("Volume Down"));

    child(m_menu, "Tray.quit")->setText(tr("Quit %1").arg(name));
    refreshIcon();
}

void TrayController::stepVolume(double db) {
    SidebarController *bar = sidebar();
    if (!bar || !m_device->connected()) return;
    if (bar->volumeMode() == QLatin1String("master")) {
        const double now = qMax(kVolumeFloor - kVolumeStep, bar->masterVolumeDb());
        bar->setMasterVolume(qMin(0.0, now + db));
    } else {
        bar->setUserVolume(qMin(0.0, bar->userVolumeDb() + db));
    }
}

// ---- Hiding and showing ---------------------------------------------------------

bool TrayController::eventFilter(QObject *watched, QEvent *event) {
    // Closing the window while the icon shows only hides it: nothing is asked,
    // the device, the hub and the polls carry on
    if (watched == m_main && event->type() == QEvent::Close && m_icon) {
        event->ignore();
        hideWindows();
        return true;
    }
    return QObject::eventFilter(watched, event);
}

void TrayController::hideWindows() {
    if (!m_main || m_hidden) return;
    m_mainVisibility = m_main->visibility();
    m_hiddenTools.clear();
    for (QWindow *w : QGuiApplication::topLevelWindows()) {
        if (w == m_main || !w->isVisible()) continue;
        if (w->type() != Qt::Window && w->type() != Qt::Dialog) continue;
        m_hiddenTools.append(w);
        w->hide();
    }
    m_main->hide();
    m_hidden = true;
    emit changed();
    refreshMenu();
}

void TrayController::minimise() {
    if (!m_main) return;
    if (!m_showInTray) setShowInTray(true);
    if (m_icon) hideWindows();
    else m_main->showMinimized();   // no tray to come back from
}

void TrayController::restore() {
    if (!m_main) return;
    const bool wasHidden = m_hidden;
    m_hidden = false;
    if (wasHidden) {
        if (m_mainVisibility == QWindow::Maximized) m_main->showMaximized();
        else if (m_mainVisibility == QWindow::FullScreen) m_main->showFullScreen();
        else m_main->showNormal();
        for (const QPointer<QWindow> &w : std::as_const(m_hiddenTools))
            if (w) w->show();
        m_hiddenTools.clear();
    } else if (m_main->visibility() == QWindow::Minimized || !m_main->isVisible()) {
        m_main->showNormal();   // -- WP QPI2 -- hidden by anyone, not only the tray
    }
    m_main->raise();
    m_main->requestActivate();
    emit changed();
    refreshMenu();
}

void TrayController::quit() { m_shell->trigger(QStringLiteral("file.quit")); }
