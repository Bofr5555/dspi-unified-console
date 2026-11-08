// WP QD12: the tray. The Mac's menu is MenuBarMode.swift:104-126.
#include "TrayController.h"

#include "DSPiBridge.h"
#include "Registrar.h"

#include <QAction>
#include <QApplication>
#include <QCloseEvent>
#include <QMenu>
#include <QPainter>
#include <QPixmap>
#include <QSessionManager>
#include <QSettings>
#include <QSystemTrayIcon>

static const Registrar::Controller<TrayController> trayRegistered("tray");

static const char *const kShowKey = "tray/showInTray";
static const char *const kStartMinimisedKey = "tray/startMinimised";
static constexpr int kRecheckMs = 5000;

static TrayController *s_instance = nullptr;
static QPointer<TrayHub> s_hub;

static bool trayAvailableNow()
{
#ifdef DSPI_SMOKE
    // Offscreen has no tray: lets a whole launch be run as if it had
    if (qEnvironmentVariableIntValue("DSPI_SMOKE_TRAY_PROBE") == 1) return true;
#endif
    return QSystemTrayIcon::isSystemTrayAvailable();
}

// The Mac fills its glyph while sharing, so the state reads at a glance: here, a
// dot on the app's icon.
static QIcon trayIcon(bool sharing)
{
    const QIcon base(QStringLiteral(":/dspi-console.svg"));
    if (!sharing) return base;
    QPixmap pixmap = base.pixmap(64, 64);
    if (pixmap.isNull()) return base;
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(Qt::black, 3));
    painter.setBrush(QColor(0x30, 0xd1, 0x58));
    painter.drawEllipse(QPoint(46, 46), 15, 15);
    painter.end();
    return QIcon(pixmap);
}

TrayController::TrayController(DSPiBridge *bridge, QObject *parent)
    : QObject(parent), m_bridge(bridge), m_probe(&trayAvailableNow)
{
    s_instance = this;
    const QSettings settings;
    m_show = settings.value(QLatin1String(kShowKey), false).toBool();
    m_startMinimised = settings.value(QLatin1String(kStartMinimisedKey), false).toBool();

    m_poll.setInterval(kRecheckMs);
    connect(&m_poll, &QTimer::timeout, this, &TrayController::recheckTray);
    connect(bridge, &DSPiBridge::statusChanged, this, &TrayController::refreshMenu);
#ifndef QT_NO_SESSIONMANAGER
    // The session is ending. Qt 5 closes every window now and takes a refusal to
    // close as a refusal to log out.
    connect(qApp, &QGuiApplication::commitDataRequest, this, [this](QSessionManager &) { m_quitting = true; });
#endif
    if (s_hub) setHub(s_hub);
    recheckTray();
}

TrayController::~TrayController()
{
    if (s_instance == this) s_instance = nullptr;
}

bool TrayController::startsHidden()
{
    const QSettings settings;
    return settings.value(QLatin1String(kShowKey), false).toBool()
        && settings.value(QLatin1String(kStartMinimisedKey), false).toBool()
        && trayAvailableNow();
}

void TrayController::attachMainWindow(QWindow *window)
{
    if (!s_instance) return;
    s_instance->m_window = window;
    if (window && !window->isVisible()) s_instance->m_hidden = true;    // started hidden
    s_instance->reconcile();
    s_instance->refreshMenu();
    emit s_instance->stateChanged();
}

void TrayController::setHub(TrayHub *hub)
{
    if (s_instance && s_hub && s_hub != hub) s_hub->disconnect(s_instance);
    s_hub = hub;
    if (!s_instance) return;
    if (hub) {
        connect(hub, &TrayHub::trayStateChanged, s_instance, &TrayController::refreshMenu, Qt::UniqueConnection);
        connect(hub, &QObject::destroyed, s_instance, &TrayController::refreshMenu);
    }
    s_instance->refreshMenu();
}

void TrayController::setShowInTray(bool on)
{
    if (on == m_show) return;
    m_show = on;
    QSettings().setValue(QLatin1String(kShowKey), on);
    emit settingsChanged();
    recheckTray();
}

void TrayController::setStartMinimised(bool on)
{
    if (on == m_startMinimised) return;
    m_startMinimised = on;
    QSettings().setValue(QLatin1String(kStartMinimisedKey), on);
    emit settingsChanged();
}

bool TrayController::trayAvailable() const
{
    return m_probe();
}

void TrayController::setAvailabilityProbe(std::function<bool()> probe)
{
    m_probe = std::move(probe);
    recheckTray();
}

void TrayController::recheckTray()
{
    // Nothing is asked of the desktop while the tray is off
    m_available = m_show && m_probe();
    reconcile();
}

// The one place that decides. A hidden window always has an icon to come back by.
void TrayController::reconcile()
{
    const bool wantIcon = m_show && m_available;
    const bool had = iconShown();
    const bool wasHidden = m_hidden;
    if (wantIcon && !had) makeIcon();
    else if (!wantIcon && had) removeIcon();
    if (m_hidden && !wantIcon) restore();
    if (m_show && !m_poll.isActive()) m_poll.start();
    else if (!m_show && m_poll.isActive()) m_poll.stop();
    if (had != iconShown() || wasHidden != m_hidden) emit stateChanged();
}

void TrayController::makeIcon()
{
    m_menu = std::make_unique<QMenu>();
    const auto named = [this](const char *name) {
        auto *action = m_menu->addAction(QString());
        action->setObjectName(QLatin1String(name));
        return action;
    };
    m_status = named("trayStatus");
    m_status->setEnabled(false);
    m_address = named("trayAddress");
    m_address->setEnabled(false);
    m_pairingCode = named("trayPairingCode");
    m_pairingCode->setEnabled(false);
    m_menu->addSeparator();
    m_open = named("trayOpen");
    connect(m_open, &QAction::triggered, this, &TrayController::restore);
    m_share = named("trayShare");
    m_share->setText(tr("Share on This Network"));
    m_share->setCheckable(true);
    connect(m_share, &QAction::triggered, this, [this](bool on) {
        if (s_hub) s_hub->setSharing(on);
        refreshMenu();      // a switch the hub turned down goes back
    });
    m_pair = named("trayPair");
    connect(m_pair, &QAction::triggered, this, [this]() {
        if (!s_hub) return;
        if (s_hub->trayState().pairingCode.isEmpty()) s_hub->allowNewClient();
        else s_hub->cancelPairing();
        refreshMenu();
    });
    m_menu->addSeparator();
    auto *quitAction = named("trayQuit");
    quitAction->setText(tr("Quit DSPi Console"));
    connect(quitAction, &QAction::triggered, this, &TrayController::quit);

    m_icon = new QSystemTrayIcon(this);
    m_iconSharing = false;
    m_icon->setIcon(trayIcon(false));
    m_icon->setContextMenu(m_menu.get());
    connect(m_icon, &QSystemTrayIcon::activated, this, [this](QSystemTrayIcon::ActivationReason reason) {
        if (reason == QSystemTrayIcon::Trigger || reason == QSystemTrayIcon::DoubleClick) restore();
    });
    refreshMenu();
    m_icon->show();

    // With only the tray left, the last window closing isn't the end
    QApplication::setQuitOnLastWindowClosed(false);
    qApp->installEventFilter(this);
}

void TrayController::removeIcon()
{
    qApp->removeEventFilter(this);
    QApplication::setQuitOnLastWindowClosed(true);
    delete m_icon;
    m_icon = nullptr;
    m_menu.reset();
    m_status = m_address = m_pairingCode = m_open = m_share = m_pair = nullptr;
}

QString TrayController::statusLine(const TrayHub::State *hub) const
{
    // -- WP QD40 -- Each line whole, for a translation
    const bool on = m_bridge->connected();
    if (!hub) return on ? tr("DSPi connected") : tr("No DSPi connected");
    if (!hub->running) return on ? tr("DSPi connected, not sharing") : tr("No DSPi connected, not sharing");
    if (hub->clients == 1) return on ? tr("DSPi connected, sharing, 1 client") : tr("No DSPi connected, sharing, 1 client");
    return (on ? tr("DSPi connected, sharing, %1 clients") : tr("No DSPi connected, sharing, %1 clients")).arg(hub->clients);
}

void TrayController::refreshMenu()
{
    if (!m_icon) return;
    TrayHub::State state;
    const bool hasHub = !s_hub.isNull();
    if (hasHub) state = s_hub->trayState();

    m_status->setText(statusLine(hasHub ? &state : nullptr));
    m_icon->setToolTip(tr("DSPi Console: %1").arg(m_status->text()));
    m_address->setText(state.address);
    m_address->setVisible(state.running && !state.address.isEmpty());
    m_pairingCode->setText(tr("Pairing code %1").arg(state.pairingCode));
    m_pairingCode->setVisible(!state.pairingCode.isEmpty());
    m_open->setText(m_hidden ? tr("Open DSPi Console") : tr("Show DSPi Console"));
    m_share->setVisible(hasHub);
    m_share->setChecked(state.sharing);
    m_pair->setVisible(hasHub && state.pinAccess);
    m_pair->setEnabled(state.running);
    m_pair->setText(state.pairingCode.isEmpty() ? tr("Allow a New Client...") : tr("Cancel Pairing"));
    if (state.running != m_iconSharing) {
        m_iconSharing = state.running;
        m_icon->setIcon(trayIcon(m_iconSharing));
    }
}

void TrayController::minimise()
{
    if (!m_window) return;
    setShowInTray(true);
    recheckTray();
    if (!iconShown() || m_hidden) return;
    m_shownAs = m_window->visibility();
    m_hidden = true;
    m_hiddenTools.clear();
    const QWindowList windows = QGuiApplication::topLevelWindows();
    for (QWindow *w : windows) {
        if (w == m_window || !w->isVisible()) continue;
        // A popup is gone once it loses the pointer, and stays gone
        if (w->type() == Qt::Window || w->type() == Qt::Tool || w->type() == Qt::Dialog) m_hiddenTools.append(w);
        w->hide();
    }
    m_window->hide();
    refreshMenu();
    emit stateChanged();
}

void TrayController::restore()
{
    if (!m_window) return;
    if (m_hidden) {
        m_hidden = false;
        if (m_shownAs == QWindow::Maximized || m_shownAs == QWindow::FullScreen) m_window->setVisibility(m_shownAs);
        else m_window->showNormal();
        const QList<QPointer<QWindow>> tools = m_hiddenTools;
        for (const QPointer<QWindow> &w : tools) {
            if (w) w->show();
        }
        m_hiddenTools.clear();
        refreshMenu();
        emit stateChanged();
    }
    m_window->raise();
    m_window->requestActivate();
}

void TrayController::quit()
{
    if (!m_window) {
        QCoreApplication::quit();
        return;
    }
    // A window that started hidden has no platform window yet, and close() on one
    // tells nobody: onClosing wouldn't run
    m_window->create();
    m_realClose = true;
    const bool closed = m_window->close();
    m_realClose = false;
    if (!closed) restore();     // main.qml is asking about unsaved changes
}

// Installed on the application while the icon is up, for three events. It reads
// main.qml's closeConfirmed rather than adding to main.qml (PLAN-CLIENTS section 1).
bool TrayController::eventFilter(QObject *watched, QEvent *event)
{
    switch (event->type()) {
    case QEvent::Quit:
        // On its way out. Qt's documentation lets a window's refusal cancel a quit,
        // though 6.11 and 5.15 were not seen to: a quit is never turned into a hide
        if (watched == QCoreApplication::instance()) m_quitting = true;
        break;
    case QEvent::Shortcut: {
        // main.qml's Quit shortcut calls root.close(), straight after this
        const QKeySequence key = static_cast<QShortcutEvent *>(event)->key();
        if (key == QKeySequence(Qt::CTRL | Qt::Key_Q) || QKeySequence::keyBindings(QKeySequence::Quit).contains(key)) {
            m_realClose = true;
            QTimer::singleShot(0, this, [this]() { m_realClose = false; });
        }
        break;
    }
    case QEvent::Close:
        if (watched == m_window && !m_quitting && !m_realClose && !m_window->property("closeConfirmed").toBool()) {
            event->ignore();
            minimise();
            return true;
        }
        break;
    default:
        break;
    }
    return QObject::eventFilter(watched, event);
}
