#ifndef HOSTAUDIOCONTROLLER_H
#define HOSTAUDIOCONTROLLER_H

#include <QObject>
#include <QThreadPool>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP FEATUI -- Settings > This Computer (FEAT3): what an agent reads with
// dspi_get_host_audio, for the person too. Is each open DSPi an output of
// this computer, the default one or not, its format, what plays to it, the
// OS volume and whether that drives the DSPi's user volume. Read through the
// MCP handle (dspi_mcp_host_audio, AiController's set) on a thread of its
// own: reading the OS can take a few seconds; it never changes anything.
class HostAudioController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The reading as the core gives it: { os, audio_system, status ("known",
    // "unknown"), why_unknown, devices: [...], default_output, other_outputs,
    // streams_known, streams_note }; empty until read
    Q_PROPERTY(QVariantMap reading READ reading NOTIFY readingChanged)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    // How to make the DSPi the default output on this computer's OS, in words
    Q_PROPERTY(QString howToDefault READ howToDefault CONSTANT)
public:
    HostAudioController(const AppContext &ctx, QObject *parent = nullptr);
    ~HostAudioController() override;
    static HostAudioController *create(QQmlEngine *, QJSEngine *);

    QVariantMap reading() const { return m_reading; }
    bool busy() const { return m_busy; }
    QString howToDefault() const;
    // Read it (again); a read on its way is not asked twice
    Q_INVOKABLE void refresh();

signals:
    void readingChanged();
    void busyChanged();

private:
    QThreadPool m_pool;
    QVariantMap m_reading;
    bool m_busy = false;
    bool m_shutDown = false;
};

#endif // HOSTAUDIOCONTROLLER_H
