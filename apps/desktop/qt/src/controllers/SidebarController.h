#ifndef SIDEBARCONTROLLER_H
#define SIDEBARCONTROLLER_H

#include <QHash>
#include <QObject>
#include <QPointer>
#include <QStringList>
#include <QVariantList>
#include <QVector>
#include <QtQml/qqmlregistration.h>
#include <cstdint>
#include <functional>
#include <memory>

#include "CoreApi.h"
#include "DeviceSession.h"
#include "Registrar.h"

class QJSEngine;
class QQmlEngine;
class QWindow;

// The sidebar's state and its writes (ContentView.swift's sidebar and the
// controls under it). One QML singleton, `SidebarController`, that the other
// main-window screens read too:
//
// - Selection: the overview (-1) or a wire channel. `selectedKind` is
//   "overview", "input" or "output"; `selectedIndex` is the input or output
//   number. A click on a row the page already covers (its own, or the other
//   half of a linked input pair) goes back to the overview, as on the Mac.
// - Curves: the pills show and toggle GraphController's visibility, which is
//   the one owner of it.
// - Linked input pairs: ChannelController owns them (inputLinks/<serial>);
//   linkedPartner() reads it for the highlight and for Paste's mirroring.
// - The channel clipboard: copy and paste a channel's bands (and, between
//   outputs, crossover, gain, delay and mute); Edit > Copy / Paste act on the
//   selected channel. -- WP CLIPQT -- The core copies and pastes
//   (dspi_copy_channel / dspi_paste_channel, its rules and its one undo step
//   "Paste Parameters"); the clip kept here is the core's JSON document.
// - Presets, source, volume and the quick strip's switches.
//
// Every write goes through apply() and DeviceModel::write().
class SidebarController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // Selection
    Q_PROPERTY(int selectedChannel READ selectedChannel NOTIFY selectionChanged)
    Q_PROPERTY(QString selectedKind READ selectedKind NOTIFY selectionChanged)
    Q_PROPERTY(int selectedIndex READ selectedIndex NOTIFY selectionChanged)
    // Rows drawn as selected: the selected channel and its linked partner
    Q_PROPERTY(int highlightedMask READ highlightedMask NOTIFY highlightChanged)
    // Rows: outputs switched off or muted (one bit per output), each output's
    // slot name ("I2S 1 L"), and whether any shown output has a derived label
    Q_PROPERTY(int inactiveOutputs READ inactiveOutputs NOTIFY deviceChanged)
    Q_PROPERTY(QStringList slotNames READ slotNames NOTIFY deviceChanged)
    Q_PROPERTY(bool derivedLabels READ derivedLabels NOTIFY deviceChanged)
    // Clipboard
    Q_PROPERTY(bool hasClipboard READ hasClipboard NOTIFY clipboardChanged)
    Q_PROPERTY(QString clipboardName READ clipboardName NOTIFY clipboardChanged)
    Q_PROPERTY(bool identifySupported READ identifySupported NOTIFY capsChanged)
    // Volume: "user" or "master", remembered (sidebar/showMaster)
    Q_PROPERTY(QString volumeMode READ volumeMode WRITE setVolumeMode NOTIFY volumeModeChanged)
    Q_PROPERTY(double userVolumeDb READ userVolumeDb NOTIFY deviceChanged)
    // The sidebar's width in px as the person left it (sidebar/width); 0 until
    // they first drag it, and the main window then uses its own
    Q_PROPERTY(int sidebarWidth READ sidebarWidth WRITE setSidebarWidth NOTIFY sidebarWidthChanged)
    Q_PROPERTY(double masterVolumeDb READ masterVolumeDb NOTIFY deviceChanged)
    // Quick strip
    Q_PROPERTY(bool crossfeedOn READ crossfeedOn NOTIFY deviceChanged)
    Q_PROPERTY(bool loudnessOn READ loudnessOn NOTIFY deviceChanged)
    Q_PROPERTY(bool levellerOn READ levellerOn NOTIFY deviceChanged)
    Q_PROPERTY(bool psybassOn READ psybassOn NOTIFY deviceChanged)
    Q_PROPERTY(bool psybassSupported READ psybassSupported NOTIFY deviceChanged)
    Q_PROPERTY(bool bypassOn READ bypassOn NOTIFY deviceChanged)
    // Action ids whose window the strip opened and that shows now
    Q_PROPERTY(QStringList openWindows READ openWindows NOTIFY openWindowsChanged)
    // Source
    Q_PROPERTY(bool sourceSupported READ sourceSupported NOTIFY deviceChanged)
    Q_PROPERTY(int source READ source NOTIFY deviceChanged)
    // [{ value, title }] in menu order
    Q_PROPERTY(QVariantList sources READ sources NOTIFY sourcesChanged)
    Q_PROPERTY(bool sourceNamesSupported READ sourceNamesSupported NOTIFY sourcesChanged)
    // Presets
    Q_PROPERTY(QStringList presetTitles READ presetTitles NOTIFY deviceChanged)
    Q_PROPERTY(bool unsaved READ unsaved NOTIFY unsavedChanged)
    Q_PROPERTY(QStringList unsavedChanges READ unsavedChanges NOTIFY unsavedChanged)
    Q_PROPERTY(bool presetBusy READ presetBusy NOTIFY presetBusyChanged)
public:
    SidebarController(const AppContext &ctx, QObject *parent = nullptr);
    static SidebarController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<SidebarController>(); }

    // Volume tapers (ContentView.swift's UserVolumeSection, Components.swift's
    // MasterVolumeTaper): slider position 0..1 to dB and back
    static constexpr double kUserMinDb = -60.0;
    static constexpr double kMasterMuteDb = -128.0;
    Q_INVOKABLE static double userPositionToDb(double position);
    Q_INVOKABLE static double userDbToPosition(double db);
    Q_INVOKABLE static double masterPositionToDb(double position);
    Q_INVOKABLE static double masterDbToPosition(double db);
    // The master readout: the step the firmware applies, "−∞" at mute
    Q_INVOKABLE static QString formatMasterDb(double db);
    Q_INVOKABLE static QString formatUserDb(double db);
    // What the device can store as a source name: printable ASCII, 31 characters
    Q_INVOKABLE static QString sanitizeSourceName(const QString &name);

    int selectedChannel() const { return state()->selected; }
    QString selectedKind() const;
    int selectedIndex() const;
    int highlightedMask() const { return m_highlighted; }
    int inactiveOutputs() const;
    QStringList slotNames() const;
    bool derivedLabels() const;
    bool hasClipboard() const { return m_hasClip; }   // -- WP CLIPQT --
    QString clipboardName() const { return m_clipName; }
    bool identifySupported() const { return state()->extras.siggenMask != 0; }
    QString volumeMode() const { return m_masterMode ? QStringLiteral("master") : QStringLiteral("user"); }
    void setVolumeMode(const QString &mode);
    int sidebarWidth() const { return m_sidebarWidth; }
    void setSidebarWidth(int width);
    double userVolumeDb() const;
    double masterVolumeDb() const;
    bool crossfeedOn() const;
    bool loudnessOn() const;
    bool levellerOn() const;
    bool psybassOn() const;
    bool psybassSupported() const;
    bool bypassOn() const;
    QStringList openWindows() const;
    bool sourceSupported() const;
    int source() const;
    QVariantList sources() const { return m_sources; }
    bool sourceNamesSupported() const { return state()->extras.namesSupported; }
    QStringList presetTitles() const;
    bool unsaved() const { return state()->extras.dirty; }
    QStringList unsavedChanges() const { return state()->extras.changes; }
    bool presetBusy() const { return state()->presetBusy; }

    // Selection
    Q_INVOKABLE void select(int channel);
    // A click on a row (the Mac's rule: a row the page covers closes it)
    Q_INVOKABLE void rowClicked(int channel);
    // The other half of a linked input pair (ChannelController's), or -1
    Q_INVOKABLE int linkedPartner(int channel) const;

    // Rows
    // Trimmed; an empty name changes nothing. False when nothing was sent.
    Q_INVOKABLE bool renameChannel(int channel, const QString &name);
    Q_INVOKABLE void copyChannel(int channel);
    Q_INVOKABLE void pasteChannel(int channel);
    Q_INVOKABLE void identifyOutput(int output);

    // Quick strip: "crossfeed", "loudness", "leveller", "psybass", "bypass"
    Q_INVOKABLE void setEffect(const QString &effect, bool on);
    // Opens the window of a menu action, or closes it when the strip opened it
    // and it shows
    Q_INVOKABLE void toggleWindow(const QString &action);

    // Volume, live while dragging and on release (the writes coalesce)
    Q_INVOKABLE void setUserVolume(double db);
    Q_INVOKABLE void setMasterVolume(double db);

    // Source
    Q_INVOKABLE void setSource(int source);
    Q_INVOKABLE QString sourceTitle(int source) const;
    Q_INVOKABLE QString builtInSourceTitle(int source) const;
    Q_INVOKABLE QString sourceName(int source) const;
    // "" restores the built-in name
    Q_INVOKABLE void renameSource(int source, const QString &name);

    // Presets. A switch or copy with unsaved changes asks first:
    // unsavedQuestion(op, slot, summary), answered with resolveUnsaved().
    Q_INVOKABLE QString presetTitle(int slot) const;  // "Empty", "Preset 3" or the name
    Q_INVOKABLE QString presetLabel(int slot) const;  // "3: Living Room"
    Q_INVOKABLE bool presetOccupied(int slot) const;
    Q_INVOKABLE void requestPresetSwitch(int slot);
    Q_INVOKABLE void requestPresetCopy(int destination);
    // answer: "accept" (save first), "alternative" (discard), "reject" (cancel)
    Q_INVOKABLE void resolveUnsaved(const QString &op, int slot, const QString &answer);
    Q_INVOKABLE void savePreset();
    Q_INVOKABLE void renamePreset(int slot, const QString &name);
    Q_INVOKABLE void setDefaultPreset();
    Q_INVOKABLE void setStartupPreset(int slot);   // -- WP LAY3b -- any slot
    Q_INVOKABLE void clearPreset(int slot);
    Q_INVOKABLE void clearAllPresets();
    // The unsaved list as the question shows it: at most 15 lines
    Q_INVOKABLE QString unsavedSummary() const;

signals:
    void selectionChanged();
    void highlightChanged();
    void clipboardChanged();
    void capsChanged();
    void volumeModeChanged();
    void sidebarWidthChanged();
    void deviceChanged();
    void openWindowsChanged();
    void sourcesChanged();
    void unsavedChanged();
    void presetBusyChanged();
    // op: "switch" or "copy"
    void unsavedQuestion(const QString &op, int slot, const QString &summary);
    // A preset operation failed: an alert's title and text
    void presetFailed(const QString &title, const QString &text);

private:
    // Facts read after each snapshot on the worker (no device traffic)
    struct Extras {
        bool dirty = false;
        QStringList changes;
        bool namesSupported = false;
        QStringList sourceNames;
        uint8_t presence = 0;
        uint16_t siggenMask = 0;
        bool capsRead = false;
    };
    // -- WP MDQT2a -- What is the device's own: the page on show, what was read
    // of it, and its preset operation under way
    struct State {
        int selected = -1;
        Extras extras;
        uint64_t capsGeneration = 0;
        bool extrasInFlight = false;
        bool extrasAgain = false;
        bool presetBusy = false;
    };
    // A paste asked of a device while the copy, of another one, is on its way
    struct WaitingPaste {
        quint64 copy;
        QPointer<DeviceSession> session;
        int channel, partner;
    };
    std::shared_ptr<State> state() const;
    std::shared_ptr<State> stateOf(DeviceSession *session) const;

    void apply(const QString &what, std::function<bool(FfiCore *)> setter, std::function<void(bool)> done = {});
    void onSnapshot();
    void readExtras();
    void applyExtras(const std::shared_ptr<State> &st, const Extras &e, bool connected);
    void pasteTo(DeviceSession *session, int channel, int partner, const std::shared_ptr<QByteArray> &clip);
    void rebuildSources();
    void updateHighlight();
    void watchLinks();
    void runPreset(const QString &what, std::function<uint8_t(FfiCore *)> op, const QString &failTitle,
                   const QString &failText);
    void loadSlot(DeviceSession *session, int slot);
    // An alert; of a device that is not the active one any more, it says which
    void presetAlert(const QPointer<DeviceSession> &session, const QString &title, const QString &text);
    void saveThenLoad(int previous, int slot);
    void copySlot(int source, int destination, const QString &preOp);
    void setPresetBusy(const std::shared_ptr<State> &st, bool busy);
    void trackWindow(const QString &action);
    static QString presetFailure(uint8_t status);
    static QString loadFailure(uint8_t status);

    AppContext m_ctx;
    std::shared_ptr<State> m_shown;   // -- WP MDQT2a -- the State last announced
    uint64_t m_askedOf = 0;           // -- WP MDQT2a -- the connection an unsaved question is about, 0 for none
    int m_highlighted = 0;
    bool m_watchingLinks = false;
    // -- WP CLIPQT -- the core's clip document (dspi.channelClip), and the
    // label the copied row showed
    QByteArray m_clip;
    QString m_clipName;
    // -- WP CLIPQTFIX -- A copy counts from the moment it is asked for: the
    // worker's own clip (`m_workerClip`, read and written only on the worker)
    // is what a paste takes, so a paste asked for right after a copy pastes
    // that copy, whatever the GUI has heard back yet (the worker runs its
    // jobs in the order posted). `m_copies` numbers the copies, so a late
    // answer to an older one changes nothing.
    bool m_hasClip = false;
    quint64 m_copies = 0;
    // -- WP MDQT2a -- That holds on the device copied from: each copy has its
    // own clip, and a paste on another device (another worker) takes the copy's
    // answer, waiting for it when it is not there yet.
    std::shared_ptr<QByteArray> m_workerClip = std::make_shared<QByteArray>();
    QPointer<DeviceSession> m_clipSession;
    bool m_clipPending = false;
    QList<WaitingPaste> m_waitingPastes;
    bool m_masterMode = false;
    int m_sidebarWidth = 0;
    QVariantList m_sources;
    QHash<QString, QPointer<QWindow>> m_windows;
};

#endif // SIDEBARCONTROLLER_H
