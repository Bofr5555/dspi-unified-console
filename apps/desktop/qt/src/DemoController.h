#ifndef DEMOCONTROLLER_H
#define DEMOCONTROLLER_H

#include <QObject>
#include <QPointer>
#include <QString>

class DSPiBridge;
class LinkController;

// WP QD7: the demo device, chosen from the running app with no hardware. Registered
// as `demo`, with qml/components/DemoBanner.qml as what the user sees: the way in
// while nothing is connected, and the label for as long as the demo is open.
//
// It needs nothing from a scan: the bridge's scans select a device only while nothing
// is connected, and the core reopens only USB devices, so an open demo stays open when
// a DSPi is plugged in or pulled out. Leaving is the user's, and then a scan may open
// what it finds. The demo is not in the bridge's device list (unless the app runs
// with --demo), since a scan selects the first of that list: it would come back
// by itself after every leave.
class DemoController : public QObject {
    Q_OBJECT
    // The simulated device is the one open, however it was opened (also --demo)
    Q_PROPERTY(bool active READ active NOTIFY changed)
    // Nothing is connected and no hub session is up: the way in is worth showing
    Q_PROPERTY(bool offered READ offered NOTIFY changed)
    // leave() would stick: under --demo the next scan reopens the demo
    Q_PROPERTY(bool leavable READ leavable NOTIFY changed)
    // Why the last enter() failed; empty after one that worked
    Q_PROPERTY(QString failure READ failure NOTIFY changed)
public:
    DemoController(DSPiBridge *bridge, QObject *parent);

    bool active() const { return m_active; }
    bool offered() const { return m_offered; }
    bool leavable() const { return m_leavable; }
    QString failure() const { return m_failure; }

    // Open the demo in place of any USB device. Refused while a hub session is up,
    // which owns the connection until it closes. In memory, so fine on the UI thread.
    Q_INVOKABLE bool enter();
    Q_INVOKABLE bool leave();

signals:
    void changed();

private:
    bool hubSessionUp() const;
    void update();

    DSPiBridge *m_bridge;
    QPointer<LinkController> m_link;
    bool m_active = false;
    bool m_offered = false;
    bool m_leavable = false;
    QString m_failure;
};

#endif // DEMOCONTROLLER_H
