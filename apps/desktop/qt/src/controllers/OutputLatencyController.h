#ifndef OUTPUTLATENCYCONTROLLER_H
#define OUTPUTLATENCYCONTROLLER_H

#include <QObject>
#include <QString>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

#include "HardwareController.h"
#include "Registrar.h"

// -- WP QP11 -- Global Parameters' Latency card (the Mac's Output Latency
// section; behaviour.md §16): the target the output ring fills to, chosen
// from the device's own options and stored on the device.
//
// The choice is held like the page's other settings and written by the save
// bar (a SaveParticipant). After a Save the status is read back: while the
// device has not applied the request (it still reports another one, or a
// change pending) the bar stays with a line saying so, the "held" state. The
// status is read again every half second for a while, and the line goes once
// the device reports it applied. Revert drops the choice and the line. A
// device without the setting (its request stalls) shows no card.
class OutputLatencyController : public QObject, public SaveParticipant {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool supported READ supported NOTIFY changed)
    // [{id, text, available}] in the device's order; a requested id the
    // table lacks shows as "-"
    Q_PROPERTY(QVariantList options READ options NOTIFY changed)
    // The id the picker shows: the held choice, or what the device has
    Q_PROPERTY(int selectedId READ selectedId NOTIFY changed)
    // The device's requested id
    Q_PROPERTY(int requestedId READ requestedId NOTIFY changed)
    Q_PROPERTY(bool edited READ edited NOTIFY changed)
    Q_PROPERTY(bool held READ held NOTIFY changed)
    // The explanation under the picker (the Mac's latencyCaption)
    Q_PROPERTY(QString caption READ caption NOTIFY changed)
public:
    OutputLatencyController(const AppContext &ctx, QObject *parent);
    static OutputLatencyController *create(QQmlEngine *, QJSEngine *) {
        return Registrar::instance<OutputLatencyController>();
    }

    bool supported() const { return m_supported && !m_options.isEmpty(); }
    QVariantList options() const;
    int selectedId() const { return m_edited ? m_choice : m_requested; }
    int requestedId() const { return m_requested; }
    bool edited() const { return m_edited; }
    bool held() const { return m_held; }
    QString caption() const;

    // Reads the status and the options again (the page shows, the source moved)
    Q_INVOKABLE void refresh();
    Q_INVOKABLE void choose(int id);

    // SaveParticipant
    bool hasPending() const override;
    void save(std::function<void(const QStringList &)> done) override;
    void revert() override;

signals:
    void changed();

private:
    struct Option {
        int id;
        bool available;
        int latencyUs;
        int effectiveUs;
    };
    struct Reading {
        bool ok = false;
        int requested = 0;
        bool pending = false;
        QList<Option> options;
    };
    static Reading read(FfiCore *c, bool withOptions);
    void take(const Reading &r, bool withOptions);
    void setHeld(bool held);
    void checkHeld();

    AppContext m_ctx;
    bool m_supported = false;
    QList<Option> m_options;
    int m_requested = 0;
    bool m_devicePending = false;
    int m_choice = 0;
    bool m_edited = false;
    bool m_held = false;
    int m_heldChecks = 0;
    QString m_serial;
    int m_lastSource = -1;
};

#endif // OUTPUTLATENCYCONTROLLER_H
