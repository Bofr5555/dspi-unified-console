#ifndef SIGGENCONTROLLER_H
#define SIGGENCONTROLLER_H

#include <QObject>
#include <QTimer>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <array>

#include "CoreApi.h"
#include "Poller.h"
#include "Registrar.h"

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
//   stop the generator.
// - Status: read every 300 ms while the generator runs, and after every
//   notification batch carrying NOTIFY_SIGGEN (a start elsewhere, completion).
// - The type table (labels, the status names, the blurbs, the fallback
//   ranges) is the Mac's; the device's capabilities override the ranges.
class SiggenController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The device: connected, has a generator, accepts writes
    Q_PROPERTY(bool connected READ connected NOTIFY capsChanged)
    Q_PROPERTY(bool supported READ supported NOTIFY capsChanged)
    Q_PROPERTY(bool controlsEnabled READ controlsEnabled NOTIFY capsChanged)
    Q_PROPERTY(int outputCount READ outputCount NOTIFY capsChanged)
    Q_PROPERTY(int multitoneMax READ multitoneMax NOTIFY capsChanged)
    // The 15 types: {id, label, name, blurb, timing, params: [{label, semantic, min, max, def}]}
    Q_PROPERTY(QVariantList types READ types NOTIFY capsChanged)
    // The draft
    Q_PROPERTY(int signalType READ signalType NOTIFY draftChanged)
    Q_PROPERTY(int channelMask READ channelMask NOTIFY draftChanged)
    Q_PROPERTY(int invertMask READ invertMask NOTIFY draftChanged)
    Q_PROPERTY(int flags READ flags NOTIFY draftChanged)
    Q_PROPERTY(double levelDb READ levelDb NOTIFY draftChanged)
    Q_PROPERTY(double durationSeconds READ durationSeconds NOTIFY draftChanged)
    Q_PROPERTY(int repeat READ repeat NOTIFY draftChanged)
    Q_PROPERTY(int gapMs READ gapMs NOTIFY draftChanged)
    Q_PROPERTY(QVariantList params READ params NOTIFY draftChanged)
    Q_PROPERTY(int timing READ timing NOTIFY draftChanged)
    Q_PROPERTY(bool walking READ walking NOTIFY draftChanged)
    // The status
    Q_PROPERTY(bool running READ running NOTIFY statusChanged)
    Q_PROPERTY(int state READ state NOTIFY statusChanged)
    Q_PROPERTY(int activeChannel READ activeChannel NOTIFY statusChanged)
    Q_PROPERTY(QString stateLabel READ stateLabel NOTIFY statusChanged)
    // What blocks Start ("" for nothing), the transport's two lines
    Q_PROPERTY(QString startBlocker READ startBlocker NOTIFY transportChanged)
    Q_PROPERTY(QString transportTitle READ transportTitle NOTIFY transportChanged)
    Q_PROPERTY(QString transportDetail READ transportDetail NOTIFY transportChanged)
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
    bool supported() const { return m_supported; }
    bool controlsEnabled() const;
    int outputCount() const;
    int multitoneMax() const { return m_caps.multitone_max; }
    QVariantList types() const;

    int signalType() const { return m_draft.signal_type; }
    int channelMask() const { return m_draft.channel_mask; }
    int invertMask() const { return m_draft.invert_mask; }
    int flags() const { return m_draft.flags; }
    double levelDb() const { return m_draft.level_db; }
    double durationSeconds() const { return m_draft.duration_ms / 1000.0; }
    int repeat() const { return m_draft.repeat; }
    int gapMs() const { return m_draft.gap_ms; }
    QVariantList params() const;
    int timing() const { return timingOf(m_draft.signal_type); }
    bool walking() const;

    bool running() const { return m_status.state != Idle; }
    int state() const { return m_status.state; }
    int activeChannel() const { return m_status.active_channel; }
    QString stateLabel() const;
    QString startBlocker() const;
    QString transportTitle() const;
    QString transportDetail() const;
    bool windowShown() const { return m_shown; }
    void setWindowShown(bool shown);

    // Edits of the draft (Mac's names)
    Q_INVOKABLE void selectType(int id);
    // off → on → inverted → off
    Q_INVOKABLE void cycleOutput(int output);
    Q_INVOKABLE void selectAll();
    Q_INVOKABLE void selectNone();
    // The field clamps to -120..0, the slider to -80..0
    Q_INVOKABLE void setLevel(double db, bool fromSlider = false);
    Q_INVOKABLE void setParam(int index, double value);
    Q_INVOKABLE void setDurationSeconds(double seconds, double minimum);
    Q_INVOKABLE void setRepeat(double count);
    Q_INVOKABLE void setGapMs(double ms);
    Q_INVOKABLE void setFlag(int flag, bool on);
    // The tone pair's presets: "smpte" or "ccif"
    Q_INVOKABLE void applyPreset(const QString &which);
    // Transport
    Q_INVOKABLE void start();
    Q_INVOKABLE void stop(bool immediate = false);
    // Space: Start while idle, the faded Stop while running
    Q_INVOKABLE void toggle();
    // An output's name: the sidebar's label, else "Out n"; enabled in the matrix
    Q_INVOKABLE QString outputName(int output) const;
    Q_INVOKABLE bool outputEnabled(int output) const;
    // The status line's numbers
    Q_INVOKABLE static QString formatElapsed(double ms);
    Q_INVOKABLE static QString formatFrequency(double hz);
    // A whole type's or one parameter's unit text
    Q_INVOKABLE static QString unitFor(int semantic);

    // For tests
    const SiggenConfig &draft() const { return m_draft; }
    const SiggenStatus &status() const { return m_status; }
    Poller::Id statusPoll() const { return m_poll; }

signals:
    void capsChanged();
    void draftChanged();
    void statusChanged();
    void transportChanged();
    void windowShownChanged();

private:
    struct Param {
        int semantic = Unused;
        double min = 0, max = 0, def = 0;
    };
    // A type's timing and parameters: the device's when it sent them, else the table's
    int timingOf(int type) const;
    Param paramOf(int type, int index) const;
    void edit(const std::function<void(SiggenConfig &)> &change);
    void onSnapshot();
    void onState(uint flags);
    void fetchAll();
    void fetchStatus();
    void setStatus(const SiggenStatus &status);
    void sendLive();
    void updatePoll();

    AppContext m_ctx;
    SiggenCaps m_caps{};
    bool m_supported = false;
    SiggenConfig m_draft{};
    SiggenStatus m_status{};
    uint64_t m_generation = 0;
    bool m_shown = false;
    Poller::Id m_poll = 0;
    // The live apply's debounce (a single shot after an edit, not a poll)
    QTimer m_live;
};

#endif // SIGGENCONTROLLER_H
