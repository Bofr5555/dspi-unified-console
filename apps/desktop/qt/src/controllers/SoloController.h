#ifndef SOLOCONTROLLER_H
#define SOLOCONTROLLER_H

#include <QObject>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP GAP2 -- "Solo this output": only one output of the active device plays,
// a listening state the core keeps (dspi_solo_*: the other outputs muted on the
// device only, never saved, not an undo step; a save, a preset load, a recall or
// a disconnect ends it). The channel menu's Solo row, the Routing inspector's
// Solo switch and the status strip's banner (SoloBanner) use it.
class SoloController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The soloed output (from 0), -1 with none
    Q_PROPERTY(int output READ output NOTIFY changed)
    // A solo can be started: a device open that takes writes
    Q_PROPERTY(bool available READ available NOTIFY changed)
public:
    SoloController(const AppContext &ctx, QObject *parent = nullptr);
    static SoloController *create(QQmlEngine *, QJSEngine *);

    int output() const { return m_output; }
    bool available() const;

    // Only `output` plays (another soloed output gives way)
    Q_INVOKABLE void solo(int output);
    // Every output plays as the user left it
    Q_INVOKABLE void end();
    // Solo `output`, or end the solo when it is the soloed one
    Q_INVOKABLE void toggle(int output);

signals:
    void changed();

private:
    void read();
    void set(int output);

    AppContext m_ctx;
    int m_output = -1;
    bool m_available = false;
};

#endif // SOLOCONTROLLER_H
