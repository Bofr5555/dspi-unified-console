#ifndef DEMOCONTROLLER_H
#define DEMOCONTROLLER_H

#include <QObject>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP QP18 -- The demo device's banner (behaviour.md §1.4) and the one that
// offers it while nothing is connected (§1.3, QD7). The demo opens only when
// asked: `--demo`, the device list, or this banner's button, never by a scan.
class DemoController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The open device is the demo (not a hub's demo over DSPi Link)
    Q_PROPERTY(bool demoOpen READ demoOpen NOTIFY changed)
    // No device open and no DSPi Link session: the banner offers the demo
    Q_PROPERTY(bool nothingConnected READ nothingConnected NOTIFY changed)
public:
    DemoController(const AppContext &ctx, QObject *parent = nullptr);
    static DemoController *create(QQmlEngine *, QJSEngine *);

    bool demoOpen() const;
    bool nothingConnected() const;

    // The device list's demo row does the same (the unsaved-changes question first)
    Q_INVOKABLE void openDemo();

signals:
    void changed();

private:
    AppContext m_ctx;
};

#endif // DEMOCONTROLLER_H
