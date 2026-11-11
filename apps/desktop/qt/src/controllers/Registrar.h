#ifndef REGISTRAR_H
#define REGISTRAR_H

#include <QObject>
#include <vector>

class Core;
class DeviceModel;
class Shell;
class QQmlEngine;

// What a controller gets when it is made: the app's base objects. All live on
// the GUI thread and outlive every controller.
struct AppContext {
    Core *core;
    DeviceModel *device;
    Shell *shell;
    QQmlEngine *engine;
};

// How a work package adds a controller, a banner or a tool window without
// editing main.cpp, the shell or another package's files. In the new
// controller's .cpp, at file scope:
//
//     #include "Registrar.h"
//     static const Registrar::Controller<HubController> hubController;
//     static const Registrar::Banner hubBanner("HubBanner", 40);
//     static const Registrar::Window hubWindow("tools.hub", "HubWindow");
//
// and add the .cpp/.h to DSPI_APP_SOURCES and the QML files to DSPI_QML_FILES
// (CMakeLists.txt) under a marker for your WP. These registrations run as
// static initialisers, so the .cpp must be compiled into the executable itself
// (as DSPI_APP_SOURCES are), not into a static library the linker could drop.
//
// Controller:
// - T's constructor is T(const AppContext &ctx, QObject *parent). It is made
//   after DeviceModel and Shell and before the QML loads, and destroyed after the
//   QML engine and before the core.
// - QML sees it as a singleton of module UConsole when T declares
//       QML_ELEMENT  (or QML_NAMED_ELEMENT(Hub))
//       QML_SINGLETON
//       static T *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<T>(); }
// - One apply path: every write goes through ctx.device->write(), called from
//   one private function of the controller that its invokables call. Never a
//   dspi_* write from elsewhere; QML calls the controller. Blocking calls go to
//   ctx.core->worker(), never on the GUI thread.
// - A menu row or shortcut: ctx.shell->addHandler("device.commit", this, [this] { ... }).
//
// Banner: a QML type of module UConsole that BannerHost stacks under the menu
// bar; `order` sorts them, lowest on top, then by name. The banner shows and
// hides itself (its own `visible`).
//
// Window: a QML type (usually a ToolWindow) that Shell opens for a menu row's
// action id, made once and raised afterwards.
namespace Registrar {

using Factory = QObject *(*)(const AppContext &ctx, QObject *parent);

struct ControllerEntry {
    const QMetaObject *meta;
    Factory make;
};
struct BannerEntry {
    const char *type;
    int order;
};
struct WindowEntry {
    const char *action;
    const char *type;
};

// Function-local: these fill during static initialisation, in no set order
std::vector<ControllerEntry> &controllers();
std::vector<BannerEntry> &banners();
std::vector<WindowEntry> &windows();

template <class T>
struct Controller {
    Controller() {
        controllers().push_back({ &T::staticMetaObject, [](const AppContext &ctx, QObject *parent) -> QObject * {
                                     return new T(ctx, parent);
                                 } });
    }
};

struct Banner {
    explicit Banner(const char *type, int order = 0) { banners().push_back({ type, order }); }
};

struct Window {
    Window(const char *action, const char *type) { windows().push_back({ action, type }); }
};

// main() calls this once, after DeviceModel and Shell and before loading the
// QML. The controllers are children of `owner`, which outlives the QML engine
// and is outlived by the core.
void install(const AppContext &ctx, QObject *owner);

// The controller of class `meta` made by install(), null before it ran.
QObject *instance(const QMetaObject *meta);

template <class T>
T *instance() {
    return static_cast<T *>(instance(&T::staticMetaObject));
}

} // namespace Registrar

#endif // REGISTRAR_H
