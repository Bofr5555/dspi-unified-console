#ifndef LAYOUTSETUPCONTROLLER_H
#define LAYOUTSETUPCONTROLLER_H

#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP UB1 -- (UXP B1, owner D5) Starting crossovers and routes from the
// speaker layout the person confirmed (Settings › Speakers), when they press
// "From Speaker Layout…" (CrossoverFromLayout): nothing happens by itself.
//
// - plan(): what the sheet offers, read from the confirmed layout, the
//   device's filters and routes and the installer's limits. One row per
//   speaker (the layout's group): its ways from the lowest up, one transition
//   between each two (woofer ◂▸ tweeter; a mid has two; a sub alone its low
//   pass), each with a frequency and a slope. A speaker whose ways are those
//   of an earlier one ("Right" after "Left") is `sameAs` it. A transition
//   starts at the frequency the outputs have (the upper way's high pass, else
//   the lower way's low pass), else at its default, and never under its floor.
// - The floor is the measurement's tweeter rule (preflight.rs tweeter_rule):
//   a tweeter's high pass at 1 kHz and 12 dB/oct at least, or the installer's
//   limits for that output where they ask more; a mid driver's 80 Hz and
//   12 dB/oct unless the limits say otherwise. apply() refuses below it, with
//   the words the sheet shows.
// - apply(): the crossovers (each output's high and low pass in the band that
//   has that pass, else the first free one) and the routes (each output of
//   the layout from the inputs its speaker's name says: L, R, C, LFE,
//   surrounds; on a stereo input the centre and the sub get L+R at −6 dB) as
//   one device write in one undo group ("From Speaker Layout"), through the
//   core's own setters, as Direct 1:1 does. Crosspoints that stay keep their
//   gain and polarity; a new one takes the output's polarity.
// - partnerFrequency(): the Crossover page's partner default. A pass chosen
//   on an output that has none yet opens at its partner's frequency (a
//   woofer's low pass at the tweeter's high pass), never under the floor.
class LayoutSetupController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // Bumped when the device, the layout or the limits change: the sheet and
    // the buttons read plan() again
    Q_PROPERTY(int revision READ revision NOTIFY changed)
    // The device may take the writes (connected, writable, not locked)
    Q_PROPERTY(bool writable READ writable NOTIFY changed)
public:
    LayoutSetupController(const AppContext &ctx, QObject *parent);
    static LayoutSetupController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<LayoutSetupController>(); }

    int revision() const { return m_revision; }
    bool writable() const;

    // { blocker: why nothing can be set ("" when it can), blockerKind
    //   ("device", "layout": none confirmed, "" for none), crossovers: the
    //   firmware has them, speakers: [{ title, sameAs, outputs, transitions:
    //   [{ lowerRole, upperRole, words, hz, family, order, floorHz,
    //   minSlopeDb, lowerOutputs, upperOutputs }] }], untouched: [words],
    //   floorText, routes: [{ input, name, outputs: [titles] }], routeNotes:
    //   [words], routed: the outputs whose routes are set }
    Q_INVOKABLE QVariantMap plan() const;
    // The sheet's values, plan().speakers' order with each row's transitions
    // [{ hz, family, order }] (a "same as" row with its model's values): ""
    // when they may be written, else why not, in the sheet's words
    Q_INVOKABLE QString check(const QVariantList &speakers) const;
    // Writes what is ticked as one undo step; "" when sent, else why not
    Q_INVOKABLE QString apply(const QVariantList &speakers, bool crossovers, bool routing);
    // The frequency a new high (or low) pass on wire channel `channel` opens
    // at: its partner way's low (high) pass in the layout, never under the
    // floor; 0 for none
    Q_INVOKABLE double partnerFrequency(int channel, bool highPass) const;

    // The tweeter rule's floor of a way on `output` (preflight.rs
    // tweeter_rule): {hz, slopeDb}, 0 / 0 for a way that has none
    struct Floor {
        double hz = 0;
        int slopeDb = 0;
    };
    static Floor floorOf(const QString &role, double profileHz, int profileSlopeDb);
    // Which input a speaker's name (or an input's) says: "L", "R", "C",
    // "LFE", "SL", "SR", "BL", "BR", or "" when it says none
    static QString sideOf(const QString &name);

signals:
    void changed();
    // After apply() sent its write: the toast's words
    void applied(const QString &text);

private:
    struct Way;
    QJsonObject layout() const;
    QVariantList speakerRows(const QJsonObject &layout, QStringList *untouched) const;
    QVariantList routePlan(const QJsonObject &layout, QStringList *notes, QVariantList *lines) const;
    Floor floorFor(const QString &role, int output) const;
    QString outputTitle(int output) const;
    QString inputName(int input) const;
    void bump();

    AppContext m_ctx;
    int m_revision = 0;
    bool m_hooked = false;
};

#endif // LAYOUTSETUPCONTROLLER_H
