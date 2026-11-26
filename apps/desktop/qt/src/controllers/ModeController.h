#ifndef MODECONTROLLER_H
#define MODECONTROLLER_H

#include <QObject>
#include <QString>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP QP18 -- The Mode choice (docs/UI-MODES.md §5): Simple or Advanced,
// remembered for the open device (by serial) or for every device on this
// console. Desktops open in Advanced. -- WP UB2 -- (UXP B2, D4) Simple opens
// on Home, a remote (SectionTable's overview/home, HomeSection.qml), and the
// dock leaves out the Master volume and Bypass EQ; every page stays reachable.
// No mode ever asks for a PIN (the Installer mode comes with the limits
// profile, and only asks when a PIN is set). Switching never writes to the device.
//
// Settings: mode/default ("simple" or "advanced") and mode/device/<serial>.
class ModeController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // "simple" or "advanced"
    Q_PROPERTY(QString mode READ mode NOTIFY changed)
    Q_PROPERTY(QString modeTitle READ modeTitle NOTIFY changed)
    // [{ id, title }]
    Q_PROPERTY(QVariantList modes READ modes CONSTANT)
    // The open device has a choice of its own
    Q_PROPERTY(bool rememberedForDevice READ rememberedForDevice NOTIFY changed)
public:
    ModeController(const AppContext &ctx, QObject *parent = nullptr);
    static ModeController *create(QQmlEngine *, QJSEngine *);

    QString mode() const;
    QString modeTitle() const;
    QVariantList modes() const;
    bool rememberedForDevice() const;

    // forEveryDevice: the console's default, and the open device's own choice forgotten
    Q_INVOKABLE void choose(const QString &mode, bool forEveryDevice);

signals:
    void changed();

private:
    static bool known(const QString &mode);
    QString deviceKey() const;

    AppContext m_ctx;
};

#endif // MODECONTROLLER_H
