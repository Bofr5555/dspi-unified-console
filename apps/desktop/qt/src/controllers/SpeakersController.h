#ifndef SPEAKERSCONTROLLER_H
#define SPEAKERSCONTROLLER_H

#include <QJsonObject>
#include <QObject>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QQmlEngine;
class QJSEngine;

// -- WP ROLEQ -- The speaker layout (MPLAN 2.2, ROLE0's store): which output
// drives what, as the person confirms it in the Getting Started step
// (SpeakersStep) and on Settings › Speakers (SpeakersPage), and as the Measure
// tables pre-fill from and write back to.
//
// - `rows` is the draft the step and the page show, one row an output of the
//   device: the stored layout's when there is one, else the core's proposal
//   (dspi_speaker_layout_propose). Edits change the draft only; Confirm keeps
//   it with confirmedByUser (dspi_speaker_layout_set).
// - The Measure tables write one output back (writeBack) and keep the
//   layout's confirmation as it was: only Confirm confirms (the web's rule,
//   ROLEW). With no stored layout the proposal is the base, kept unconfirmed.
// - `status`: "disconnected", "none" (only the proposal), "unconfirmed",
//   "imported" (an unconfirmed layout this console did not write: it came with
//   an imported configuration, review RA F1), "confirmed". What this console
//   wrote last is speakers/written/<serial hex> = its updatedMs.
// - Where the layouts live is the core's: <AppData>/speaker-layouts, which
//   main.cpp names for every core of the set (MDQT1, dspi_core_set_config).
class SpeakersController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QVariantList rows READ rows NOTIFY rowsChanged)
    Q_PROPERTY(QString status READ status NOTIFY storedChanged)
    // When the stored layout was set, ms since the epoch; 0 for none
    Q_PROPERTY(qint64 stamp READ stamp NOTIFY storedChanged)
    // The draft differs from the stored layout (or there is none)
    Q_PROPERTY(bool edited READ edited NOTIFY rowsChanged)
    Q_PROPERTY(bool editable READ editable NOTIFY editableChanged)
    Q_PROPERTY(bool loaded READ loaded NOTIFY storedChanged)
    // The proposal's remarks, shown while nothing is stored
    Q_PROPERTY(QStringList notes READ notes NOTIFY storedChanged)
    // The last outcome in words, and whether it was a failure
    Q_PROPERTY(QString said READ said NOTIFY saidChanged)
    Q_PROPERTY(bool saidError READ saidError NOTIFY saidChanged)
public:
    SpeakersController(const AppContext &ctx, QObject *parent);
    static SpeakersController *create(QQmlEngine *, QJSEngine *);

    QVariantList rows() const { return m_rows; }
    QString status() const;
    qint64 stamp() const;
    bool edited() const;
    bool editable() const;
    bool loaded() const { return m_loaded; }
    QStringList notes() const { return m_notes; }
    QString said() const { return m_said; }
    bool saidError() const { return m_saidError; }

    // The stored layout as the core gave it, {} for none
    QJsonObject stored() const { return m_stored; }

    // Reads the stored layout and the proposal again (the page, the step and
    // Measure call it when they show)
    Q_INVOKABLE void reload();
    // The draft
    Q_INVOKABLE void setIncluded(int output, bool on);
    Q_INVOKABLE void setRole(int output, const QString &role);
    Q_INVOKABLE void setSpeaker(int output, const QString &speaker);
    Q_INVOKABLE void setGroup(int output, int group);
    // The draft back to the proposal
    Q_INVOKABLE void useSuggestion();
    // Keeps the draft as the confirmed layout
    Q_INVOKABLE void confirm();
    // An output's role, speaker and group as the stored layout (else the
    // proposal) has them; "" role for an output in neither
    Q_INVOKABLE QString roleOf(int output) const;
    Q_INVOKABLE QVariantMap rowOf(int output) const;
    // A Measure table's edit of one output, kept in the layout with its
    // confirmation as it was; group < 0 or an empty speaker keep the layout's
    Q_INVOKABLE void writeBack(int output, const QString &role, const QString &speaker = QString(), int group = -1);
    // The banner's words for `status`, elsewhere (Measure) or on the page
    // itself; "" when the layout is confirmed
    Q_INVOKABLE QString bannerText(bool here) const;
    // Settings › Speakers in the Settings window
    Q_INVOKABLE void openSettings();

signals:
    void rowsChanged();
    void editableChanged();
    void saidChanged();
    // The stored layout changed; `origin` is "measure" for a Measure table's
    // write-back, "confirm", or "read" (a reload found it changed)
    void storedChanged(const QString &origin);

private:
    void onSnapshot();
    void setStored(const QJsonObject &layout, const QString &origin);
    void rebuild();
    void say(const QString &text, bool error);
    void setField(int output, const QString &field, const QVariant &value);
    QJsonObject draftLayout() const;
    static void remember(const QString &serial, const QJsonObject &kept);
    static bool imported(const QString &serial, const QJsonObject &layout);
    QVariantList outputs() const;

    AppContext m_ctx;
    QString m_serial;
    bool m_loaded = false;
    QJsonObject m_stored;     // {} for none
    QJsonObject m_proposal;   // {} until read
    QStringList m_notes;
    QVariantList m_rows;
    QVariantList m_outputs;   // [{output, label}] of the device
    QString m_said;
    bool m_saidError = false;
    bool m_editable = false;
    bool m_touched = false;     // the person changed the draft since it was built
    quint64 m_generation = 0;   // a reload's answer for an older device is dropped
};

#endif // SPEAKERSCONTROLLER_H
