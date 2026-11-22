#ifndef SIGGENCONTROLLER_H
#define SIGGENCONTROLLER_H

#include <QObject>
#include <QTimer>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <array>
#include <functional>
#include <memory>

#include "CoreApi.h"
#include "Poller.h"
#include "Registrar.h"

class DeviceSession;
class QQmlEngine;
class QJSEngine;

// -- WP QP9 -- The signal generator (the Mac's TestSignalsView and its view
// model half): the draft configuration, the device's capabilities and the
// generator's status.
//
// - The draft lives here for the app's session (no settings key, as on the
//   Mac): Sine 1 kHz on outputs 1 and 2 at -20 dBFS at first, replaced on
//   each connect by the configuration the device has applied when that has a
//   channel. Nothing goes to the device until Start; while the generator runs
//   an edit is sent again 350 ms after the last change (the firmware fades
//   over to it).
// - Start sends the configuration and SIGGEN_CTL_START, Stop SIGGEN_CTL_STOP
//   (faded) or SIGGEN_CTL_STOP_NOW, all through DeviceModel::write, so a
//   read-only device sends nothing. Closing the window or quitting does not
//   stop the generator. -- WP MDQT2c -- Leaving the device does (another one
//   becomes active, it is closed or replaced): what was started here is
//   stopped there, faded. A measurement's generator is not this one's.
// - Status: read every 300 ms while the generator runs, and after every
//   notification batch carrying NOTIFY_SIGGEN (a start elsewhere, completion).
// - -- WP MDQT2c -- The draft, the capabilities and the status are per device
//   (the session's stash); what is shown is the active one's.
// - The type table (labels, the status names, the blurbs, the fallback
//   ranges) is the Mac's; the device's capabilities override the ranges.
class SiggenController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The device: connected, has a generator, accepts writes
    Q_PROPERTY(bool connected READ connected NOTIFY generatorChanged)
    Q_PROPERTY(bool available READ available NOTIFY generatorChanged)
    Q_PROPERTY(bool controlsEnabled READ controlsEnabled NOTIFY generatorChanged)
    Q_PROPERTY(int outputCount READ outputCount NOTIFY generatorChanged)
    Q_PROPERTY(int mostTones READ mostTones NOTIFY generatorChanged)
    // The 15 types: {id, label, name, blurb, timing, params: [{label, semantic, min, max, def}]}
    Q_PROPERTY(QVariantList types READ types NOTIFY generatorChanged)
    // The draft
    Q_PROPERTY(int signalType READ signalType NOTIFY draftEdited)
    Q_PROPERTY(int channelMask READ channelMask NOTIFY draftEdited)
    Q_PROPERTY(int invertMask READ invertMask NOTIFY draftEdited)
    Q_PROPERTY(int flags READ flags NOTIFY draftEdited)
    Q_PROPERTY(double levelDb READ levelDb NOTIFY draftEdited)
    Q_PROPERTY(double durationSeconds READ durationSeconds NOTIFY draftEdited)
    Q_PROPERTY(int repeat READ repeat NOTIFY draftEdited)
    Q_PROPERTY(int gapMs READ gapMs NOTIFY draftEdited)
    Q_PROPERTY(QVariantList params READ params NOTIFY draftEdited)
    Q_PROPERTY(int timing READ timing NOTIFY draftEdited)
    Q_PROPERTY(bool walking READ walking NOTIFY draftEdited)
    // The status
    Q_PROPERTY(bool playing READ playing NOTIFY playbackChanged)
    Q_PROPERTY(int phase READ phase NOTIFY playbackChanged)
    Q_PROPERTY(int walkOutput READ walkOutput NOTIFY playbackChanged)
    Q_PROPERTY(QString phaseText READ phaseText NOTIFY playbackChanged)
    // What blocks Start ("" for nothing), the transport's two lines
    Q_PROPERTY(QString startBlocker READ startBlocker NOTIFY captionsChanged)
    Q_PROPERTY(QString transportTitle READ transportTitle NOTIFY captionsChanged)
    Q_PROPERTY(QString transportDetail READ transportDetail NOTIFY captionsChanged)
    // The window shows (status is read while it runs either way)
    Q_PROPERTY(bool windowShown READ windowShown WRITE setWindowShown NOTIFY windowShownChanged)

public:
    // The firmware's ids (siggen.h), not in the C header
    enum Type { Sine = 0, Square, White, Pink, SweepLog, SweepLin, SweepStep, Impulse, ClicksAlt, Polarity,
                ToneBurst, TonePair, Multitone, Isp, ChannelId };
    enum State { Idle = 0, FadeIn, Run, Gap, FadeOut };
    enum StopReason { StopNone = 0, StopHost, StopCompleted, StopPreset, StopReconfig };
    enum Timing { Continuous = 0, Sweep, Pattern };
    enum Semantic { Unused = 0, FreqHz, Ms, Cycles, Count, Ratio, PatternChoice };
    Q_ENUM(Type)
    Q_ENUM(State)
    Q_ENUM(Timing)
    Q_ENUM(Semantic)

    static constexpr double kLevelMin = -120, kLevelMax = 0;
    static constexpr double kSliderMin = -80;
    static constexpr double kDefaultLevel = -20;
    static constexpr int kStatusPollMs = 300;
    static constexpr int kLiveApplyMs = 350;
    static constexpr int kNotWalking = 0xFF;

    SiggenController(const AppContext &ctx, QObject *parent = nullptr);
    static SiggenController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<SiggenController>(); }

    bool connected() const;
    bool available() const;
    bool controlsEnabled() const;
    int outputCount() const;
    int mostTones() const;
    QVariantList types() const;

    int signalType() const;
    int channelMask() const;
    int invertMask() const;
    int flags() const;
    double levelDb() const;
    double durationSeconds() const;
    int repeat() const;
    int gapMs() const;
    QVariantList params() const;
    int timing() const;
    bool walking() const;

    bool playing() const;
    int phase() const;
    int walkOutput() const;
    QString phaseText() const;
    QString startBlocker() const;
    QString transportTitle() const;
    QString transportDetail() const;
    bool windowShown() const;
    void setWindowShown(bool shown);

    // Edits of the draft (Mac's names)
    Q_INVOKABLE void selectType(int id);
    // off → on → inverted → off
    Q_INVOKABLE void stepOutput(int output);
    Q_INVOKABLE void allOutputs();
    Q_INVOKABLE void noOutputs();
    // The field clamps to -120..0, the slider to -80..0
    Q_INVOKABLE void setLevel(double db, bool fromSlider = false);
    Q_INVOKABLE void setParameter(int index, double value);
    Q_INVOKABLE void setDurationSeconds(double seconds, double minimum);
    Q_INVOKABLE void setRepeat(double count);
    Q_INVOKABLE void setGapMs(double ms);
    Q_INVOKABLE void setOption(int flag, bool on);
    // The tone pair's presets: "smpte" or "ccif"
    Q_INVOKABLE void applyPreset(const QString &which);
    // Transport
    Q_INVOKABLE void play();
    Q_INVOKABLE void stopPlaying(bool immediate = false);
    // Space: Start while idle, the faded Stop while playing
    Q_INVOKABLE void toggle();
    // Identify (an output's menu): the channel-ID blips on that output alone
    Q_INVOKABLE void playIdent(int output);
    // An output's name: the sidebar's label, else "Out n"; enabled in the matrix
    Q_INVOKABLE QString outputName(int output) const;
    Q_INVOKABLE bool outputEnabled(int output) const;
    // The status line's numbers
    Q_INVOKABLE static QString formatElapsed(double ms);
    Q_INVOKABLE static QString formatFrequency(double hz);
    // A whole type's or one parameter's unit text
    Q_INVOKABLE static QString unitFor(int semantic);

    // For tests
    const SiggenConfig &draft() const { return state()->draft; }
    const SiggenStatus &status() const { return state()->status; }
    Poller::Id statusPoll() const;

signals:
    void generatorChanged();
    void draftEdited();
    void playbackChanged();
    void captionsChanged();
    void windowShownChanged();

private:
    struct Param {
        int semantic = Unused;
        double min = 0, max = 0, def = 0;
    };
    // A type's timing and parameters: the device's when it sent them, else the table's
    int timingOf(int type) const;
    Param paramOf(int type, int index) const;
    // -- WP MDQT2c -- one device's
    struct Device {
        SiggenCaps caps{};
        bool supported = false;
        SiggenConfig draft{};
        SiggenStatus status{};
        uint64_t generation = 0;
        // Started (or edited while it ran) from here, and not seen to end since;
        // `seen`: it was seen playing since the last start
        bool ours = false;
        bool seen = false;
        QString serial;   // the device it was started on
        int starts = 0;   // Starts on their way
        Device();
    };
    using StatePtr = std::shared_ptr<Device>;
    StatePtr state() const;
    bool shown(const StatePtr &st) const { return st == state(); }
    void started(const StatePtr &st);
    void stopOn(DeviceSession *session);
    void sendStart(const SiggenConfig &cfg);
    void edit(const std::function<void(SiggenConfig &)> &change);
    void onSnapshot();
    void onState(uint flags);
    void fetchAll();
    void fetchStatus();
    void fetchStatus(DeviceSession *session, const StatePtr &st);
    void setStatus(const StatePtr &st, const SiggenStatus &status);
    void sendLive();
    void updatePoll();

    AppContext m_ctx;
    StatePtr m_announced;   // -- WP MDQT2c -- the one the last change notices were for
    bool m_shown = false;
    Poller::Id m_poll = 0;
    // The live apply's debounce (a single shot after an edit, not a poll)
    QTimer m_live;
};

#endif // SIGGENCONTROLLER_H
