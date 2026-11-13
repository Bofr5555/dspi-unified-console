#ifndef READONLYCONTROLLER_H
#define READONLYCONTROLLER_H

#include <QObject>
#include <QString>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP QP16 -- What the device refuses, as the banners say it:
// - Read-only mode (behaviour.md §14): firmware of an older or newer wire
//   format than this app writes. Everything it has is shown and nothing is
//   written (DeviceModel refuses every write but clearing the clip marks and
//   entering the bootloader); the banner offers the firmware window.
// - The delay refusal (behaviour.md §15): a delay the core refused (outside
//   the device's delay line), in the core's own words, until the next
//   accepted delay, a dismissal or another device.
class ReadOnlyController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool active READ active NOTIFY changed)
    // The firmware is newer than this app understands (else older)
    Q_PROPERTY(bool newer READ newer NOTIFY changed)
    Q_PROPERTY(QString text READ text NOTIFY changed)
    Q_PROPERTY(QString actionText READ actionText NOTIFY changed)
    Q_PROPERTY(bool actionShown READ actionShown NOTIFY changed)
    Q_PROPERTY(QString delayRefusal READ delayRefusal NOTIFY delayRefusalChanged)
public:
    ReadOnlyController(const AppContext &ctx, QObject *parent = nullptr);
    static ReadOnlyController *create(QQmlEngine *, QJSEngine *);

    bool active() const;
    bool newer() const;
    QString text() const;
    QString actionText() const;
    bool actionShown() const;
    QString delayRefusal() const { return m_delayRefusal; }

    // The banner's button: the firmware window
    Q_INVOKABLE void openDetails();
    Q_INVOKABLE void dismissDelayRefusal();

signals:
    void changed();
    void delayRefusalChanged();

private:
    void setDelayRefusal(const QString &text);

    AppContext m_ctx;
    QString m_delayRefusal;
    quint64 m_generation = 0;
};

#endif // READONLYCONTROLLER_H
