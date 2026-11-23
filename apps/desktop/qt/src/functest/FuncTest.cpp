#include "FuncTest.h"

#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "Shell.h"

#include <QDir>
#include <QGuiApplication>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLocalServer>
#include <QLocalSocket>
#include <QMetaMethod>
#include <QMetaProperty>
#include <QQuickItem>
#include <QQuickWindow>

#include <functional>

static const Registrar::Controller<FuncTest> funcTest;

FuncTest::FuncTest(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    const QString path = qEnvironmentVariable("DSPI_FUNCTEST_SOCKET");
    if (path.isEmpty()) return;
    m_server = new QLocalServer(this);
    m_server->setSocketOptions(QLocalServer::UserAccessOption);
    QLocalServer::removeServer(path);
    if (!m_server->listen(path)) {
        qWarning("functest: %s", qPrintable(m_server->errorString()));
        return;
    }
    connect(m_server, &QLocalServer::newConnection, this, [this] {
        while (QLocalSocket *s = m_server->nextPendingConnection()) {
            connect(s, &QLocalSocket::disconnected, s, &QObject::deleteLater);
            connect(s, &QLocalSocket::readyRead, this, [this, s] {
                while (s->canReadLine()) {
                    const QJsonObject answer = handle(QJsonDocument::fromJson(s->readLine()).object());
                    s->write(QJsonDocument(answer).toJson(QJsonDocument::Compact) + '\n');
                }
            });
        }
    });
}

QObject *FuncTest::find(const QString &name) const {
    if (name == QLatin1String("DeviceRegistry")) return m_ctx.registry;
    if (name == QLatin1String("DeviceModel")) return m_ctx.device;
    // -- WP DESREV -- the shell (trigger, openWindow, closeWindow), for screenshots of dialogs
    if (name == QLatin1String("Shell")) return m_ctx.shell;
    for (const Registrar::ControllerEntry &c : Registrar::controllers())
        if (name == QLatin1String(c.meta->className())) return Registrar::instance(c.meta);
    // -- WP FUNC2b -- a visible item by its objectName ("TopBar.preset"): what a screen shows
    // (the visual tree, as the smoke driver walks it)
    const std::function<QQuickItem *(QQuickItem *)> walk = [&](QQuickItem *at) -> QQuickItem * {
        if (!at || !at->isVisible()) return nullptr;
        if (at->objectName() == name) return at;
        for (QQuickItem *child : at->childItems())
            if (QQuickItem *found = walk(child)) return found;
        return nullptr;
    };
    for (QWindow *w : QGuiApplication::topLevelWindows())
        if (auto *quick = qobject_cast<QQuickWindow *>(w); quick && quick->isVisible())
            if (QQuickItem *found = walk(quick->contentItem())) return found;
    return nullptr;
}

static QJsonObject failed(const QString &why) { return { { QStringLiteral("ok"), false }, { QStringLiteral("error"), why } }; }

static QJsonObject done(const QVariant &value = {}) {
    return { { QStringLiteral("ok"), true }, { QStringLiteral("value"), QJsonValue::fromVariant(value) } };
}

QJsonObject FuncTest::handle(const QJsonObject &command) {
    const QString op = command.value(QLatin1String("op")).toString();
    if (op == QLatin1String("quit")) {
        QMetaObject::invokeMethod(qApp, &QCoreApplication::quit, Qt::QueuedConnection);
        return done();
    }
    if (op == QLatin1String("page")) {
        // A page of Modern, as a click in its rail and on its tab
        QVariant ok;
        if (m_ctx.shell->pageHost())
            QMetaObject::invokeMethod(m_ctx.shell->pageHost(), "go", Q_RETURN_ARG(QVariant, ok),
                                      Q_ARG(QVariant, command.value(QLatin1String("section")).toString()),
                                      Q_ARG(QVariant, command.value(QLatin1String("tab")).toString()));
        return done(ok);
    }
    if (op == QLatin1String("grab")) {
        const QDir dir(command.value(QLatin1String("dir")).toString());
        const QString prefix = command.value(QLatin1String("prefix")).toString();
        QStringList files;
        for (QWindow *w : QGuiApplication::topLevelWindows()) {
            auto *quick = qobject_cast<QQuickWindow *>(w);
            if (!quick || !quick->isVisible()) continue;
            const QString name = quick->objectName().isEmpty() ? QStringLiteral("window") : quick->objectName();
            const QString file = dir.filePath(QStringLiteral("%1%2-%3.png").arg(prefix, name).arg(files.size()));
            if (quick->grabWindow().save(file)) files << file;
        }
        return done(files);
    }
    QObject *on = find(command.value(QLatin1String("on")).toString());
    if (!on) return failed(QStringLiteral("no such controller"));
    const QMetaObject *meta = on->metaObject();
    if (op == QLatin1String("get") || op == QLatin1String("set")) {
        QByteArray name = command.value(QLatin1String("prop")).toString().toUtf8();
        // -- WP FUNC2b -- "presets.startupMode": a property of the object a property holds
        for (int dot = name.indexOf('.'); dot > 0; dot = name.indexOf('.')) {
            on = on->property(name.left(dot).constData()).value<QObject *>();
            if (!on) return failed(QStringLiteral("no such object"));
            meta = on->metaObject();
            name = name.mid(dot + 1);
        }
        if (meta->indexOfProperty(name.constData()) < 0) return failed(QStringLiteral("no such property"));
        if (op == QLatin1String("get")) return done(on->property(name.constData()));
        return on->setProperty(name.constData(), command.value(QLatin1String("value")).toVariant())
                   ? done()
                   : failed(QStringLiteral("not set"));
    }
    if (op != QLatin1String("call")) return failed(QStringLiteral("no such op"));
    const QByteArray name = command.value(QLatin1String("method")).toString().toUtf8();
    const QVariantList args = command.value(QLatin1String("args")).toArray().toVariantList();
    for (int i = 0; i < meta->methodCount(); ++i) {
        const QMetaMethod m = meta->method(i);
        if (m.name() != name || m.parameterCount() != args.size() || args.size() > 6) continue;
        QVariantList values = args;
        QGenericArgument a[6];
        bool ok = true;
        for (int p = 0; p < values.size(); ++p) {
            const QMetaType type = m.parameterMetaType(p);
            if (type.id() != QMetaType::QVariant) ok = ok && values[p].convert(type);
            a[p] = type.id() == QMetaType::QVariant ? QGenericArgument("QVariant", &values[p])
                                                    : QGenericArgument(type.name(), values[p].constData());
        }
        if (!ok) continue;
        const QMetaType returns = m.returnMetaType();
        if (returns.id() == QMetaType::Void)
            return m.invoke(on, Qt::DirectConnection, a[0], a[1], a[2], a[3], a[4], a[5]) ? done() : failed(QStringLiteral("not called"));
        QVariant value(returns);
        const bool called = m.invoke(on, Qt::DirectConnection, QGenericReturnArgument(returns.name(), value.data()), a[0], a[1],
                                     a[2], a[3], a[4], a[5]);
        return called ? done(value) : failed(QStringLiteral("not called"));
    }
    return failed(QStringLiteral("no such method"));
}
