#include "Shell.h"

#include "AppIdentity.h"

#include <QCoreApplication>
#include <QDesktopServices>
#include <QFontDatabase>
#include <QGuiApplication>
#include <QJSEngine>
#include <QJSValue>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QScreen>
#include <QStyleHints>
#include <QUrl>
#include <QWindow>

namespace {
Shell *s_instance = nullptr;
}

Shell::Shell(QQmlEngine *engine, QObject *parent) : QObject(parent), m_engine(engine) {
    s_instance = this;
    connect(QGuiApplication::styleHints(), &QStyleHints::colorSchemeChanged, this, &Shell::darkSchemeChanged);
    // The rows the shell handles itself
    addHandler(QStringLiteral("file.quit"), this, [this]() { quit(); });
    addHandler(QStringLiteral("help.projectGithub"), this,
               [this]() { openUrl(AppIdentity::projectUrl()); });
    addHandler(QStringLiteral("help.firmwareGithub"), this,
               [this]() { openUrl(AppIdentity::firmwareUrl()); });
}

Shell::~Shell() {
    if (s_instance == this) s_instance = nullptr;
}

void Shell::releaseWindows() {
    for (const QPointer<QWindow> &w : std::as_const(m_windows))
        if (w) delete w.data();
    m_windows.clear();
}

Shell *Shell::instance() { return s_instance; }

Shell *Shell::create(QQmlEngine *, QJSEngine *) {
    QJSEngine::setObjectOwnership(s_instance, QJSEngine::CppOwnership);
    return s_instance;
}

void Shell::addHandler(const QString &action, QObject *receiver, std::function<void()> handler) {
    if (m_handlers.contains(action) && m_handlers.value(action).receiver)
        qFatal("Shell: the action \"%s\" has a handler already", qPrintable(action));
    m_handlers.insert(action, { receiver, std::move(handler) });
}

void Shell::addWindow(const QString &action, const QString &type) {
    if (m_windowTypes.contains(action)) qFatal("Shell: the action \"%s\" has a window already", qPrintable(action));
    m_windowTypes.insert(action, type);
}

void Shell::setBanners(const QStringList &types) {
    if (types == m_banners) return;
    m_banners = types;
    emit bannersChanged();
}

void Shell::setColorSchemeOverride(Qt::ColorScheme scheme) {
    if (scheme == m_schemeOverride) return;
    m_schemeOverride = scheme;
    emit darkSchemeChanged();
}

bool Shell::darkScheme() const {
    const Qt::ColorScheme scheme =
        m_schemeOverride != Qt::ColorScheme::Unknown ? m_schemeOverride : QGuiApplication::styleHints()->colorScheme();
    return scheme != Qt::ColorScheme::Light;
}

QString Shell::monospaceFamily() { return QFontDatabase::systemFont(QFontDatabase::FixedFont).family(); }

bool Shell::handles(const QString &action) const {
    const auto it = m_handlers.constFind(action);
    return (it != m_handlers.cend() && it->receiver) || m_windowTypes.contains(action);
}

void Shell::trigger(const QString &action) {
    // -- WP QTSENDER -- the gate first: a refused id does nothing
    if (!allows(action)) {
        qInfo("Shell: \"%s\" is refused on this device", qPrintable(action));
        return;
    }
    emit triggered(action);
    const auto it = m_handlers.constFind(action);
    if (it != m_handlers.cend() && it->receiver) {
        // A copy: the handler may add or remove handlers
        const std::function<void()> run = it->run;
        run();
        return;
    }
    if (m_windowTypes.contains(action)) {
        // -- WP LAY0 -- a panel goes to its page while the shell has pages
        if (m_pageHost) {
            const QVariantMap row = panel(action);
            QVariant shown;
            if (!row.isEmpty()
                && QMetaObject::invokeMethod(m_pageHost.data(), "showPanel", Q_RETURN_ARG(QVariant, shown),
                                             Q_ARG(QVariant, row))
                && (shown.canConvert<QJSValue>() ? shown.value<QJSValue>().toBool() : shown.toBool()))
                return;
        }
        openWindow(action);
        return;
    }
    qInfo("Shell: nothing handles \"%s\" yet", qPrintable(action));
    m_lastUnhandled = action;
    emit lastUnhandledChanged();
}

QWindow *Shell::openWindow(const QString &action) {
    const QString type = m_windowTypes.value(action);
    if (type.isEmpty()) return nullptr;
    QPointer<QWindow> &w = m_windows[action];
    if (!w) {
        // -- WP LAY0 -- a panel opens in a PanelWindow named as the registered type
        const QVariantMap row = panel(action);
        QQmlComponent component(m_engine);
        component.loadFromModule("UConsole", row.isEmpty() ? type : QStringLiteral("PanelWindow"));
        QObject *o = row.isEmpty() ? component.create()
                                   : component.createWithInitialProperties(
                                         { { QStringLiteral("panel"), row }, { QStringLiteral("objectName"), type } });
        if (!o) {
            qWarning("Shell: the window %s did not load: %s", qPrintable(type), qPrintable(component.errorString()));
            return nullptr;
        }
        w = qobject_cast<QWindow *>(o);
        if (!w) {
            qWarning("Shell: %s is not a Window", qPrintable(type));
            delete o;
            return nullptr;
        }
        QJSEngine::setObjectOwnership(w, QJSEngine::CppOwnership);
        // -- WP QTSENDER -- ToolWindow greys its content by it
        if (w->metaObject()->indexOfProperty("actionId") >= 0) w->setProperty("actionId", action);
    }
    w->show();
    w->raise();
    w->requestActivate();
    return w;
}

// -- WP LAY0 --
QVariantList Shell::panels() const {
    QObject *table = m_engine ? m_engine->singletonInstance<QObject *>("UConsole", "PanelTable") : nullptr;
    if (!table) return {};
    const QVariant rows = table->property("panels");
    return rows.canConvert<QJSValue>() ? rows.value<QJSValue>().toVariant().toList() : rows.toList();
}

QVariantMap Shell::panel(const QString &action) const {
    for (const QVariant &row : panels()) {
        const QVariantMap map = row.toMap();
        if (map.value(QStringLiteral("action")).toString() == action) return map;
    }
    return {};
}

void Shell::setPageHost(QObject *host) {
    if (host == m_pageHost) return;
    disconnect(m_pageHostGone);
    m_pageHost = host;
    // The QPointer is null already when `destroyed` comes
    if (host)
        m_pageHostGone = connect(host, &QObject::destroyed, this, [this]() {
            m_pageHost = nullptr;
            emit pageHostChanged();
        });
    emit pageHostChanged();
}

void Shell::openUrl(const QString &url) {
    const QUrl u(url);
    // Only web pages: a menu row never opens a local file or another scheme
    if (u.scheme() != QLatin1String("https")) return;
    QDesktopServices::openUrl(u);
}

void Shell::quit() {
    // -- WP QP15 -- the quit guard first
    if (m_quitGuardReceiver && m_quitGuard) {
        m_quitGuard([] { QCoreApplication::quit(); });
        return;
    }
    QCoreApplication::quit();
}

// -- WP QP15 --
void Shell::setQuitGuard(QObject *receiver, std::function<void(std::function<void()>)> guard) {
    m_quitGuardReceiver = receiver;
    m_quitGuard = std::move(guard);
}

// -- WP QTSENDER --
void Shell::setActionGate(QObject *receiver, std::function<bool(const QString &)> gate) {
    m_gateReceiver = receiver;
    m_gate = std::move(gate);
}

bool Shell::allows(const QString &action) const { return !m_gateReceiver || !m_gate || m_gate(action); }

// -- WP QTPOP --
QSize Shell::fitCap(const QWindow *window) const {
    QScreen *screen = window && window->screen() ? window->screen() : QGuiApplication::primaryScreen();
    QSize room = screen ? screen->availableGeometry().size() : QSize(1 << 24, 1 << 24);
    for (const QWindow *w : QGuiApplication::topLevelWindows()) {
        if (w->objectName() == QLatin1String("MainWindow") && w->isVisible()) {
            room = room.boundedTo(w->size());
            break;
        }
    }
    return QSize(qMax(0, room.width() - 2 * kFitMargin), qMax(0, room.height() - 2 * kFitMargin));
}

QSize Shell::fitWindow(QWindow *window, int width, int height) const {
    if (!window) return {};
    const QSize cap = fitCap(window), now = window->size();
    const auto side = [](int has, int want, int most) { return want > has ? qMax(has, qMin(want, most)) : want; };
    const QSize size(side(now.width(), width, cap.width()), side(now.height(), height, cap.height()));
    if (size == now) return now;
    window->resize(size);
    // Wayland places windows itself and ignores this
    QScreen *screen = window->screen();
    if (!screen) return size;
    const QRect avail = screen->availableGeometry();
    const QMargins frame = window->frameMargins();
    const QSize got = window->size();
    QPoint at = window->position();
    at.setX(qMax(avail.left() + frame.left(), qMin(at.x(), avail.right() + 1 - frame.right() - got.width())));
    at.setY(qMax(avail.top() + frame.top(), qMin(at.y(), avail.bottom() + 1 - frame.bottom() - got.height())));
    if (at != window->position()) window->setPosition(at);
    return size;
}
