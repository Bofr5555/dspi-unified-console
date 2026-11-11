#include "Shell.h"

#include "AppIdentity.h"

#include <QCoreApplication>
#include <QDesktopServices>
#include <QGuiApplication>
#include <QJSEngine>
#include <QQmlComponent>
#include <QQmlEngine>
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

bool Shell::handles(const QString &action) const {
    const auto it = m_handlers.constFind(action);
    return (it != m_handlers.cend() && it->receiver) || m_windowTypes.contains(action);
}

void Shell::trigger(const QString &action) {
    emit triggered(action);
    const auto it = m_handlers.constFind(action);
    if (it != m_handlers.cend() && it->receiver) {
        // A copy: the handler may add or remove handlers
        const std::function<void()> run = it->run;
        run();
        return;
    }
    if (m_windowTypes.contains(action)) {
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
        QQmlComponent component(m_engine);
        component.loadFromModule("UConsole", type);
        QObject *o = component.create();
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
    }
    w->show();
    w->raise();
    w->requestActivate();
    return w;
}

void Shell::openUrl(const QString &url) {
    const QUrl u(url);
    // Only web pages: a menu row never opens a local file or another scheme
    if (u.scheme() != QLatin1String("https")) return;
    QDesktopServices::openUrl(u);
}

void Shell::quit() { QCoreApplication::quit(); }
