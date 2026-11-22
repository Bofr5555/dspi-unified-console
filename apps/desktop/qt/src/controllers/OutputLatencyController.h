#ifndef OUTPUTLATENCYCONTROLLER_H
#define OUTPUTLATENCYCONTROLLER_H

#include <QObject>
#include <QPointer>
#include <QString>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <memory>

#include "DeviceSession.h"
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
    Q_PROPERTY(bool supported READ supported NOTIFY changed FINAL)
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

    bool supported() const { return state()->supported && !state()->options.isEmpty(); }
    QVariantList options() const;
    int selectedId() const { return state()->edited ? state()->choice : state()->requested; }
    int requestedId() const { return state()->requested; }
    bool edited() const { return state()->edited; }
    bool held() const { return state()->held; }
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
    // -- WP MDQT2a -- The setting, the choice and the wait for it are the device's own
    struct State {
        bool supported = false;
        QList<Option> options;
        int requested = 0;
        bool devicePending = false;
        int choice = 0;
        bool edited = false;
        bool held = false;
        int heldChecks = 0;
        QString serial;
        int lastSource = -1;
    };
    std::shared_ptr<State> state() const;
    void onSnapshot();
    static Reading read(FfiCore *c, bool withOptions);
    void take(const std::shared_ptr<State> &st, const Reading &r, bool withOptions);
    void setHeld(const std::shared_ptr<State> &st, bool held);
    // Reads on `session`, whichever device shows
    void checkHeld(const QPointer<DeviceSession> &session, const std::shared_ptr<State> &st);

    AppContext m_ctx;
    std::shared_ptr<State> m_shown;   // -- WP MDQT2a -- the State last announced
};

#endif // OUTPUTLATENCYCONTROLLER_H
