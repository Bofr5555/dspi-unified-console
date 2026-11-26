#ifndef UNSAVEDGUARD_H
#define UNSAVEDGUARD_H

#include <QElapsedTimer>
#include <QList>
#include <QObject>
#include <QPointer>
#include <QStringList>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "Registrar.h"

class DeviceSession;
class QJSEngine;
class QQmlEngine;
class QQuickWindow;

// The unsaved-changes question (M42) on the paths that leave the device's
// live state behind: picking another device (the device list: USB, the demo, a
// hub's device), quitting and closing the main window. The preset popup's own
// switch and Copy to stay SidebarController's.
//
// Each question first settles: it waits until every write queued before it
// has run, and for an edit that came back less than 100 ms ago it waits out
// the rest of those 100 ms (not over a hub session, whose echoes arrive with
// the writes), then asks the core (dspi_preset_dirty, dspi_preset_changes).
// Nothing unsaved: the path goes on without a question.
//
// A question while the main window is hidden in the tray (a quit from the
// tray's menu) brings the window back first.
//
// Answers, as on the Mac: Save stores the live state in the active slot and
// goes on (a failed save stops a device switch; quitting then asks "Quit
// anyway?"); Discard goes on; Cancel stops. The host (files/FilesHost.qml)
// shows `question` and calls answer().
//
// -- WP MDQT2b -- Several open devices: each has its own preset and changes.
// Closing a device (a tab, the Open devices list) asks about that one, active
// or not, and saves into its own active slot; quitting and closing the main
// window ask about every open device in turn, the active one first (a Cancel
// stops there). `deviceTitle` names the device asked about while more than one
// is open.
//
// -- WP UA3 -- Save All (the Modern top bar's Save ▾): one question about
// every open device with unsaved changes, each named with its preset, then a
// save into each one's active slot (askSaveAll(), saveAllQuestion(),
// answerSaveAll()). And after a Keep (a safe apply's, an auto-tune's) on a
// device with unsaved changes, a notice says that it is not saved on the
// device yet and offers Save to Device.
class UnsavedGuard : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // A question (or its save) is under way; another path asking meanwhile is
    // dropped (-- WP LAY7 -- with a notice, and the question comes forward)
    Q_PROPERTY(bool asking READ asking NOTIFY askingChanged)
    // Closing the main window asks first. Not while the close only hides it
    // (QGuiApplication::quitOnLastWindowClosed() false, or this set false by a
    // tray that hides the window instead), as the Mac asks nothing when it
    // hides to the menu bar.
    Q_PROPERTY(bool askOnClose READ askOnClose WRITE setAskOnClose NOTIFY askOnCloseChanged)
    // -- WP MDQT2b -- The device the question is about, "" while it is the only one open
    Q_PROPERTY(QString deviceTitle READ deviceTitle NOTIFY askingChanged)
public:
    UnsavedGuard(const AppContext &ctx, QObject *parent = nullptr);
    static UnsavedGuard *create(QQmlEngine *, QJSEngine *);

    bool asking() const { return m_asking; }
    bool askOnClose() const { return m_askOnClose; }
    void setAskOnClose(bool ask);

    // Runs `proceed` once the live state may be left (see the class comment),
    // while `ctx` exists. `op`: "device", "quit" or "close".
    void ask(const QString &op, QObject *ctx, std::function<void()> proceed);
    // ask() through the one instance; without one, or with no `op` (the same
    // device again), `proceed` runs at once
    static void before(const QString &op, QObject *ctx, std::function<void()> proceed);
    // -- WP MDQT2b -- ask() about `session`'s device, before it is closed
    // (op "device"); without an instance `proceed` runs at once
    static void beforeClosing(DeviceSession *session, QObject *ctx, std::function<void()> proceed);
    QString deviceTitle() const { return m_title; }

    // The question's answer: "accept" (Save), "alternative" (Discard) or
    // "reject" (Cancel)
    Q_INVOKABLE void answer(const QString &answer);
    // After a failed save before quitting: "accept" quits anyway, anything else stays
    Q_INVOKABLE void answerSaveFailed(const QString &answer);
    // -- WP QP16 -- The settings question's answer: "accept" (Discard and
    // Switch) goes on to the preset's question, anything else stops
    Q_INVOKABLE void answerSettings(const QString &answer);
    // -- WP LAY7 -- The busy question's answer: "accept" goes on to the presets'
    // questions, anything else stays
    Q_INVOKABLE void answerBusy(const QString &answer);

    // The core's lines as the question lists them: at most 15, then a count
    static QString summary(const QStringList &lines);

    // -- WP UA3 -- Save All: reads every open device, then saveAllQuestion()
    // with those that have unsaved changes (none: a notice says so)
    Q_INVOKABLE void askSaveAll();
    // Its answer: "accept" saves each one into its active slot
    Q_INVOKABLE void answerSaveAll(const QString &answer);
    // "preset 1 “Demo”", or "preset 1" for a slot without a name
    static QString presetText(int slot, const QString &name);

signals:
    void askingChanged();
    void askOnCloseChanged();
    // The host shows the question; `summary` is the list of changes
    void question(const QString &op, const QString &summary);
    // The save before `op` failed: an alert, or for "quit" a question
    // (Quit / Cancel) answered with answerSaveFailed()
    void saveFailed(const QString &op, const QString &title, const QString &text);
    // -- WP QP16 -- Before a device switch: Settings holds changes for this
    // device that were not saved (HardwareController.pending), which the
    // switch discards (the Mac's "Unsaved Settings Changes"); answered with
    // answerSettings()
    void settingsQuestion(const QString &title, const QString &text);
    // -- WP LAY7 -- (review RB B5) Before quitting: a firmware update or a core
    // voltage calibration runs on an open device, shown or not
    // (DevicesController::pinnedBy), which quitting breaks off; answered with
    // answerBusy()
    void busyQuestion(const QString &title, const QString &text);
    // -- WP UA3 -- Save All's question (TopBar shows it), answered with answerSaveAll()
    void saveAllQuestion(const QString &title, const QString &text);
    // Save All ended: how many devices were saved, and the failures in words
    void savedAll(int saved, const QStringList &failed);

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void setAsking(bool asking);
    // -- WP MDQT2b -- the devices asked about, in turn
    void begin(const QString &op, QList<DeviceSession *> targets, QObject *ctx, std::function<void()> proceed);
    void askNext();
    QStringList busyTexts() const;   // -- WP LAY7 --
    void refuseSecond(const QString &op);   // -- WP LAY7 --
    void settleThenRead();
    void read();
    void proceed();
    void finish();
    void wire(QQuickWindow *window);
    void bringBack();
    bool cleanForSure() const;
    // -- WP UA3 --
    struct SaveTarget {
        QPointer<DeviceSession> session;
        int slot = -1;
        QString name;
        QStringList changes;
    };
    void collectSaveAll(QList<QPointer<DeviceSession>> left);
    void saveNext();
    void watchKeeps();
    void keptOn(DeviceSession *session);

    AppContext m_ctx;
    bool m_asking = false;
    bool m_askOnClose = true;
    QString m_op;
    QPointer<QObject> m_ctxObject;
    std::function<void()> m_proceed;
    QElapsedTimer m_sinceChange;   // since the last snapshot of a write or notification
    QPointer<QQuickWindow> m_window;
    bool m_closePass = false;      // the next close of the main window goes through
    bool m_quitting = false;       // the person chose to quit: every close goes through
    // -- WP MDQT2b --
    QList<QPointer<DeviceSession>> m_targets;   // still to be asked about
    QPointer<DeviceSession> m_target;           // the one asked about now
    QString m_title;
    // -- WP LAY7 -- a question is on show (not settling, not saving)
    bool m_shown = false;
    // -- WP UA3 -- Save All: the devices read so far, then those still to save
    QList<SaveTarget> m_saveAll;
    int m_savedAll = 0;
    QStringList m_saveAllFailed;
    QString m_guardPhase;   // the safe apply's phase last seen
};

#endif // UNSAVEDGUARD_H
