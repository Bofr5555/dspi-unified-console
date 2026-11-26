#ifndef SPEAKERSCONTROLLER_H
#define SPEAKERSCONTROLLER_H

#include <QJsonObject>
#include <QObject>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <memory>

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
// - -- WP MDQT2c -- Each open device has its own draft, layout and words; what is
//   shown is the active one's. A Confirm or a write-back answered after a switch
//   is its device's. `storedChanged` is the active one's (another device
//   becoming active is a "read").
// - -- WP ADETQ -- (AUTODES 3, 5.1) A layout found by the speaker detection
//   (SpeakerLayout v2: rows' `source`, the `detected` record) is
//   "autoDetected" until Confirm: its roles guard the outputs meanwhile. Each
//   row carries a pill (whose word it is, how sure a detection was) with its
//   reason; an edited role is the person's ("you"). The app's own detection
//   (detect(), after the page's question) runs on the active device's Measure
//   session, with its microphone.
// - -- WP ROLES -- (owner 2026-11-23) A layout an AI agent set from the person's
//   words (rows `stated`, the `stated` record: the words, the checks) is
//   "stated", shown as "set by AI from your words, not checked" until Confirm;
//   an output the words leave out is "unknown" (a tweeter), a stated row whose
//   top-fall check held like a tweeter's is "tweeter-like".
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
    // -- WP LAY5b -- `said` when it is about a Measure table's edit, else ""
    // (Confirm's is not for under that table)
    Q_PROPERTY(QString saidMeasure READ saidMeasure NOTIFY saidChanged)
    // -- WP ADETQ -- When the kept layout was confirmed, detected or kept, in
    // words ("Detected by AI, not confirmed, today at 14:02."); "" for none
    Q_PROPERTY(QString stampText READ stampText NOTIFY storedChanged)
    // -- WP ADETQ -- The stored layout's detection: { by, atMs, stopped, summary,
    // placement, rows: [{ output, label, klass, confidence, heard, topFall,
    // fLow, lowest }] }; {} for none
    Q_PROPERTY(QVariantMap detection READ detection NOTIFY storedChanged)
    // -- WP ADETQ -- The app's own detection: running, why it cannot start ("" when
    // it can), where it stands, and its question's heading and words
    Q_PROPERTY(bool detecting READ detecting NOTIFY detectChanged)
    Q_PROPERTY(QString detectBlocker READ detectBlocker NOTIFY detectChanged)
    Q_PROPERTY(QString detectText READ detectText NOTIFY detectChanged)
    Q_PROPERTY(QString detectHeading READ detectHeading NOTIFY detectChanged)
    Q_PROPERTY(QString detectQuestion READ detectQuestion NOTIFY detectChanged)
    // -- WP UB7 -- (UXP B7) The draft's groups follow its speakers' names: the
    // same name is one group, another name another group (unnamed rows aside)
    Q_PROPERTY(bool groupsFollowNames READ groupsFollowNames NOTIFY rowsChanged)
    // -- WP UB7 -- The layout Measure and Auto-tune use (the stored one, else the
    // suggestion) in one line: "L: woofer Out 1, tweeter Out 2 · R: … · confirmed"
    Q_PROPERTY(QString summary READ summary NOTIFY storedChanged)
public:
    SpeakersController(const AppContext &ctx, QObject *parent);
    static SpeakersController *create(QQmlEngine *, QJSEngine *);

    // -- WP ADETQ -- each with its pill: { pill, pillKind, pillReason, source }
    // -- WP UB7 -- and `groupName`: the speakers measured with it ("L", "L + Sub"),
    // "" for a row left out
    QVariantList rows() const;
    QString status() const;
    qint64 stamp() const;
    bool edited() const;
    bool editable() const;
    bool loaded() const { return state()->loaded; }
    QStringList notes() const { return state()->notes; }
    QString said() const { return state()->said; }
    bool saidError() const { return state()->saidError; }
    QString saidMeasure() const { return state()->saidInMeasure ? state()->said : QString(); }
    // -- WP ADETQ --
    QString stampText() const;
    QVariantMap detection() const;
    bool detecting() const;
    QString detectBlocker() const;
    QString detectText() const;
    QString detectHeading() const;
    QString detectQuestion() const;
    bool groupsFollowNames() const;   // -- WP UB7 --
    QString summary() const;          // -- WP UB7 --

    // The stored layout as the core gave it, {} for none
    QJsonObject stored() const { return state()->stored; }

    // Reads the stored layout and the proposal again (the page, the step and
    // Measure call it when they show)
    Q_INVOKABLE void reload();
    // The draft
    Q_INVOKABLE void setIncluded(int output, bool on);
    Q_INVOKABLE void setRole(int output, const QString &role);
    Q_INVOKABLE void setSpeaker(int output, const QString &speaker);
    Q_INVOKABLE void setGroup(int output, int group);
    // -- WP UB7 -- (UXP B7) A speaker's name sets the group: a name another row has
    // joins its group, a new name gets a group of its own (setSpeaker and
    // setIncluded do it); followNames() does it for every row again
    Q_INVOKABLE void followNames();
    // The draft back to the proposal
    Q_INVOKABLE void useSuggestion();
    // Keeps the draft as the confirmed layout
    Q_INVOKABLE void confirm();
    // An output's role, speaker and group as the stored layout (else the
    // proposal) has them; "" role for an output in neither
    Q_INVOKABLE QString roleOf(int output) const;
    Q_INVOKABLE QVariantMap rowOf(int output) const;
    // -- WP SHOWFIX -- (finding 8) an output's pill as the Speakers page shows it
    // ({pill, pillKind, pillReason}; empty words for none), for the Overview's cards
    Q_INVOKABLE QVariantMap pillFor(int output) const;
    // A Measure table's edit of one output, kept in the layout with its
    // confirmation as it was; group < 0 or an empty speaker keep the layout's
    Q_INVOKABLE void writeBack(int output, const QString &role, const QString &speaker = QString(), int group = -1);
    // The banner's words for `status`, elsewhere (Measure) or on the page
    // itself; "" when the layout is confirmed
    Q_INVOKABLE QString bannerText(bool here) const;
    // Settings › Speakers in the Settings window
    Q_INVOKABLE void openSettings();
    // -- WP ADETQ -- The app's own speaker detection, after the page's question
    // (AUTODES 4.4): on the active device, with the Measure view's microphone
    Q_INVOKABLE void detect();
    Q_INVOKABLE void stopDetect();
    // -- WP ADETQ -- The detection heard nothing from `output` (and the layout
    // leaves it out): the channel inspector says so
    Q_INVOKABLE bool heardNothing(int output) const;

signals:
    void rowsChanged();
    void editableChanged();
    void saidChanged();
    // The stored layout changed; `origin` is "measure" for a Measure table's
    // write-back, "confirm", or "read" (a reload found it changed)
    void storedChanged(const QString &origin);
    void detectChanged();   // -- WP ADETQ --

private:
    // -- WP MDQT2c -- one device's, in its session's stash: a draft stays with it
    struct State {
        QString serial;
        bool loaded = false;
        QJsonObject stored;     // {} for none
        QJsonObject proposal;   // {} until read
        QStringList notes;
        QVariantList rows;
        QVariantList outputs;   // [{output, label}] of the device
        QString said;
        bool saidError = false;
        bool saidInMeasure = false;   // -- WP LAY5b --
        bool touched = false;     // the person changed the draft since it was built
        quint64 generation = 0;   // a reload's answer for an older device is dropped
    };
    using StatePtr = std::shared_ptr<State>;
    StatePtr state() const;
    bool shown(const StatePtr &st) const { return st == state(); }
    void onSnapshot();
    void updateEditable();   // -- FIX016 -- (R4)
    void setStored(const StatePtr &st, const QJsonObject &layout, const QString &origin);
    void rebuild(const StatePtr &st);
    void say(const StatePtr &st, const QString &text, bool error, bool measure = false);
    void setField(int output, const QString &field, const QVariant &value);
    void settleGroup(int output);   // -- WP UB7 --
    static QVariantMap rowOf(const State &st, int output);
    QVariantMap pillOf(const State &st, const QVariantMap &row) const;   // -- WP ADETQ --
    QJsonObject draftLayout() const;
    static void remember(const QString &serial, const QJsonObject &kept);
    static bool imported(const QString &serial, const QJsonObject &layout);
    QVariantList outputs() const;

    AppContext m_ctx;
    StatePtr m_announced;   // the one the last change notices were for
    bool m_editable = false;
    bool m_detectAsked = false;    // -- WP ADETQ -- the page asked for a detection: its failures are said here
    bool m_wasDetecting = false;   // -- WP ADETQ --
};

#endif // SPEAKERSCONTROLLER_H
