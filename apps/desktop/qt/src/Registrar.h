#ifndef REGISTRAR_H
#define REGISTRAR_H

#include <QObject>
#include <QQmlContext>
#include <QStringList>
#include <algorithm>
#include <vector>

class DSPiBridge;

// How a work package adds a controller or a banner without editing DSPiBridge,
// main.cpp or main.qml. In the new controller's .cpp, at file scope:
//
//     #include "Registrar.h"
//     static const Registrar::Controller<HubController> hubRegistered("hub");
//     static const Registrar::Banner hubBanner("qrc:/qml/components/HubBanner.qml");
//
// and add the .cpp to DSPI_APP_SOURCES (CMakeLists.txt) and the QML file to
// resources.qrc, under a marker for your WP.
//
// Controller:
// - T's constructor is T(DSPiBridge *bridge, QObject *parent). QML sees the object
//   as the context property `name`, as it sees `rta` or `stats`. A name already
//   taken stops the app at startup, so the first run shows it.
// - It is made before main.qml loads, after the bridge, and destroyed after the
//   QML engine and before the bridge. Register a QML type from the constructor,
//   if it needs one.
// - Show something only when another WP's controller is there:
//   `visible: typeof hub !== "undefined"`.
// - One apply path: every write to the device goes through one private function
//   of the controller, which the invokable setters call. Never a dspi_* write
//   from elsewhere; QML calls setters. Undo, if it ever comes, hooks in there.
//   Blocking calls stay off the UI thread.
//
// Banner: see qml/BannerHost.qml. `order` sorts them, lowest on top, then by URL.
namespace Registrar {

struct ControllerEntry {
    const char *name;
    QObject *(*make)(DSPiBridge *bridge, QObject *parent);
};
struct BannerEntry {
    const char *url;
    int order;
};

// Function-local: these fill during static initialisation, in no set order
inline std::vector<ControllerEntry> &controllers() {
    static std::vector<ControllerEntry> list;
    return list;
}
inline std::vector<BannerEntry> &banners() {
    static std::vector<BannerEntry> list;
    return list;
}

template <class T>
struct Controller {
    explicit Controller(const char *name) {
        controllers().push_back({ name, [](DSPiBridge *bridge, QObject *parent) -> QObject * {
                                     return new T(bridge, parent);
                                 } });
    }
};

struct Banner {
    explicit Banner(const char *url, int order = 0) { banners().push_back({ url, order }); }
};

// main.cpp calls this once, after its own context properties and before loading
// main.qml. `owner` outlives the engine and is outlived by the bridge.
inline void install(QQmlContext *context, DSPiBridge *bridge, QObject *owner) {
    for (const ControllerEntry &c : controllers()) {
        if (context->contextProperty(QString::fromLatin1(c.name)).isValid())
            qFatal("Registrar: the context property name \"%s\" is taken", c.name);
        context->setContextProperty(QString::fromLatin1(c.name), c.make(bridge, owner));
    }
    std::vector<BannerEntry> sorted = banners();
    std::sort(sorted.begin(), sorted.end(), [](const BannerEntry &a, const BannerEntry &b) {
        if (a.order != b.order) return a.order < b.order;
        return QString::fromLatin1(a.url) < QString::fromLatin1(b.url);
    });
    QStringList urls;
    for (const BannerEntry &b : sorted) urls.append(QString::fromLatin1(b.url));
    context->setContextProperty(QStringLiteral("registeredBanners"), urls);
}

} // namespace Registrar

#endif // REGISTRAR_H
