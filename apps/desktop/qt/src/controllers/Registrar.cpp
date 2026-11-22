#include "Registrar.h"

#include "DeviceRegistry.h"   // -- WP MDQT1 --
#include "Shell.h"

#include <QHash>
#include <QJSEngine>
#include <QStringList>
#include <algorithm>

// -- WP MDQT1 --
DeviceSession *AppContext::activeSession() const { return registry ? registry->active() : nullptr; }

namespace Registrar {

std::vector<ControllerEntry> &controllers() {
    static std::vector<ControllerEntry> list;
    return list;
}
std::vector<BannerEntry> &banners() {
    static std::vector<BannerEntry> list;
    return list;
}
std::vector<WindowEntry> &windows() {
    static std::vector<WindowEntry> list;
    return list;
}

static QHash<const QMetaObject *, QObject *> &made() {
    static QHash<const QMetaObject *, QObject *> map;
    return map;
}

void install(const AppContext &ctx, QObject *owner) {
    for (const ControllerEntry &c : controllers()) {
        if (made().contains(c.meta)) qFatal("Registrar: %s is registered twice", c.meta->className());
        QObject *o = c.make(ctx, owner);
        QJSEngine::setObjectOwnership(o, QJSEngine::CppOwnership);
        made().insert(c.meta, o);
        QObject::connect(o, &QObject::destroyed, [meta = c.meta]() { made().remove(meta); });
    }

    std::vector<BannerEntry> sorted = banners();
    std::sort(sorted.begin(), sorted.end(), [](const BannerEntry &a, const BannerEntry &b) {
        if (a.order != b.order) return a.order < b.order;
        return QString::fromLatin1(a.type) < QString::fromLatin1(b.type);
    });
    QStringList types;
    for (const BannerEntry &b : sorted) types.append(QString::fromLatin1(b.type));
    ctx.shell->setBanners(types);

    for (const WindowEntry &w : windows()) ctx.shell->addWindow(QString::fromLatin1(w.action), QString::fromLatin1(w.type));
}

QObject *instance(const QMetaObject *meta) { return made().value(meta, nullptr); }

} // namespace Registrar
