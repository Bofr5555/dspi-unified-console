#ifndef TOOLSCONTROLLER_H
#define TOOLSCONTROLLER_H

#include <QObject>
#include <QString>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "CoreApi.h"
#include "Poller.h"
#include "Registrar.h"

class QJSEngine;

// -- WP QP8 -- The controller of the Subharmonic Synthesizer, the Tube Modeller,
// the Stereo Upmixer and the output limiters' settings (the Mac's
// SubharmonicSynthView, TubeModellerView, UpmixerView, OutputLimiterSettings
// and their ToolParameters).
//
// Reads come from DeviceModel's snapshot; the core mirrors the firmware's own
// rules (a tube type loads its character, a character edit drops the type to
// Custom, linked limiters move together), so what the windows show is what the
// device holds. Every write goes through apply(), keyed by its target
// ("subharm low", "output limiter threshold 3"): a drag's live values and its
// release replace each other in the queue. Parameters are named by key:
//   subharm: enabled mask low high top boost select depth hold ceiling link
//   tube:    enabled mask type drive bias asym hardness sag rectifier xfmr
//            damping resonance mix trim
//   upmix:   enabled center surround strength width threshold attack release
//            detector delay hpf lpf decorrelation presence
//
// The live readings poll only while their window shows (windowShown): the
// subharmonic headroom (after each change) and its sub meter (10 Hz), the
// upmixer's telemetry (about 16 Hz). The tube's glow follows the base's meter
// poll. Closing the subharmonic window switches solo off, as on the Mac.
//
// Limiter edits are part of the output configuration: each one calls
// HardwareController.beginOutputEdit() first, so the save bar offers to keep
// or revert it in Independent mode.
class ToolsController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(int revision READ revision NOTIFY toolsChanged)
    Q_PROPERTY(bool connected READ connected NOTIFY toolsChanged)
    Q_PROPERTY(bool writable READ writable NOTIFY toolsChanged)
    Q_PROPERTY(int numOutputs READ numOutputs NOTIFY toolsChanged)
    Q_PROPERTY(int pdmOutput READ pdmOutput NOTIFY toolsChanged)

    Q_PROPERTY(bool subharmSupported READ subharmSupported NOTIFY toolsChanged)
    Q_PROPERTY(bool subharmExtended READ subharmExtended NOTIFY toolsChanged)
    Q_PROPERTY(bool subharmSolo READ subharmSolo NOTIFY toolsChanged)
    Q_PROPERTY(double subharmHeadroom READ subharmHeadroom NOTIFY headroomChanged)
    Q_PROPERTY(QVariantList subharmMeter READ subharmMeter NOTIFY subharmMeterChanged)
    Q_PROPERTY(QVariantList subharmStartingPoints READ subharmStartingPoints CONSTANT)

    Q_PROPERTY(bool tubeSupported READ tubeSupported NOTIFY toolsChanged)
    Q_PROPERTY(QVariantList tubeTypes READ tubeTypes CONSTANT)
    Q_PROPERTY(QVariantList tubeRectifiers READ tubeRectifiers CONSTANT)
    Q_PROPERTY(QVariantList tubeStartingPoints READ tubeStartingPoints CONSTANT)
    Q_PROPERTY(double tubeGlow READ tubeGlow NOTIFY tubeGlowChanged)

    Q_PROPERTY(bool upmixSupported READ upmixSupported NOTIFY toolsChanged)
    Q_PROPERTY(QVariantMap upmixStatus READ upmixStatus NOTIFY upmixStatusChanged)

    Q_PROPERTY(bool limiterSupported READ limiterSupported NOTIFY toolsChanged)
    // -- WP P1QT2 -- PRO1 S2: the outputs that are on, above -10 dB of gain and
    // without a limiter (the core's outputs_without_limiter), and its warning row
    Q_PROPERTY(QVariantList unprotectedOutputs READ unprotectedOutputs NOTIFY unprotectedChanged)
    Q_PROPERTY(QString unprotectedText READ unprotectedText NOTIFY unprotectedChanged)
public:
    // -- WP P1QT2 -- Protect all outputs' ceiling, and the warning's gain
    static constexpr float kProtectCeilingDb = -1.0f;
    static constexpr float kUnprotectedAboveDb = -10.0f;

    ToolsController(const AppContext &ctx, QObject *parent);
    QVariantList unprotectedOutputs() const { return m_unprotected; }   // -- WP P1QT2 --
    QString unprotectedText() const;                                    // -- WP P1QT2 --
    static ToolsController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<ToolsController>(); }

    int revision() const { return m_reads; }
    bool connected() const;
    bool writable() const;
    int numOutputs() const;
    int pdmOutput() const { return numOutputs() - 1; }

    // ---- Subharmonic synthesizer (wire V29, extended V30) ----
    bool subharmSupported() const;
    bool subharmExtended() const;
    bool subharmSolo() const;
    double subharmHeadroom() const { return m_headroom; }
    QVariantList subharmMeter() const { return m_subMeter; }
    static QVariantList subharmStartingPoints();
    Q_INVOKABLE double subharm(const QString &key) const;
    Q_INVOKABLE void setSubharm(const QString &key, double value);
    Q_INVOKABLE void setSubharmSolo(bool soloed);
    Q_INVOKABLE void applySubharmStartingPoint(int index);
    Q_INVOKABLE void toggleSubharmOutput(int output);

    // ---- Tube modeller (wire V31) ----
    bool tubeSupported() const;
    // {name, shortName, style, pushPull} per type, index 0 = Custom
    static QVariantList tubeTypes();
    // {name, depth, attack, release} per rectifier
    static QVariantList tubeRectifiers();
    static QVariantList tubeStartingPoints();
    double tubeGlow() const { return m_glow; }
    Q_INVOKABLE double tube(const QString &key) const;
    Q_INVOKABLE void setTube(const QString &key, double value);
    Q_INVOKABLE void applyTubeStartingPoint(int index);
    Q_INVOKABLE void toggleTubeOutput(int output);
    // The drawing family of a tube type (the Mac's TubeFamily.of)
    Q_INVOKABLE static int tubeFamily(int type);

    // ---- Stereo upmixer (wire V26, RP2350) ----
    bool upmixSupported() const;
    // {active, parked, correlation, balance, center, ls, rs}
    QVariantMap upmixStatus() const { return m_upmixStatus; }
    Q_INVOKABLE double upmix(const QString &key) const;
    Q_INVOKABLE void setUpmix(const QString &key, double value);

    // ---- Output limiters (wire V32) ----
    bool limiterSupported() const;
    // {enabled, threshold, release, group}
    Q_INVOKABLE QVariantMap limiter(int output) const;
    Q_INVOKABLE void setLimiterEnabled(int output, bool on);
    Q_INVOKABLE void setLimiterThreshold(int output, double db);
    Q_INVOKABLE void setLimiterRelease(int output, double ms);
    Q_INVOKABLE void setLimiterGroup(int output, int group);
    // The popover's batch actions (M52), and "Protect all outputs" (PRO1 S2)
    Q_INVOKABLE void copyLimiterToAll(int output);
    Q_INVOKABLE void linkLimiterPairs();
    Q_INVOKABLE void unlinkLimiters();
    Q_INVOKABLE void switchLimitersOff();
    Q_INVOKABLE void protectAllOutputs();
    // "Linked with Out 2." / "Not linked." / "No other outputs in group 3."
    Q_INVOKABLE QString limiterLinkSummary(int output) const;
    // Outputs 1+2 in group 1, 3+4 in group 2 and so on up to group 4; the PDM sub unlinked
    Q_INVOKABLE QVariantList stereoPairGroups() const;

    // ---- Masks and names ----
    Q_INVOKABLE int allOutputsMask() const;
    Q_INVOKABLE int subOnlyMask() const;
    Q_INVOKABLE int withoutSubMask() const;
    Q_INVOKABLE QString outputName(int output) const;

    // A tool window shows or hides: "subharmonic", "tube", "upmixer", "limiter"
    Q_INVOKABLE void windowShown(const QString &window, bool shown);

    // ---- Drawings (normalised: x and y 0..1, y from the top) ----
    // The subharmonic band graph: {bell: [x, y, ...], blocks: [{lo, hi, points}], sources: [{x0, x1}],
    //   ceilingY, zeroY, gridX: [...], gridY: [...], gridDb: [...], freqX: [...], freqs: [...]}
    Q_INVOKABLE QVariantMap subharmGraph(double low, double high, double top, double boost, double ceiling) const;
    // The tube's transfer curve: {curve: [x, y, ...], clipPos, clipNeg (x where the input clips, or NaN),
    //   second, third (harmonics, dB, -120 = none)}
    Q_INVOKABLE QVariantMap tubeGraph(double drive, double bias, double asym, double hardness, double mix,
                                      double trim) const;
    // The output stage's lift: {bell, top} in dB at the damping factor
    Q_INVOKABLE static QVariantMap transformerLift(double damping);

    // The Mac's formulas, for the tests
    static double subharmBellDb(double boostDb, double hz);
    static double subharmSubDb(double bandDb, double boostDb, double hz);
    static double tubeOutput(double drive, double bias, double asym, double hardness, double mix, double trim, double x);
    static void tubeHarmonics(double drive, double bias, double asym, double hardness, double mix, double trim,
                              double &second, double &third);
    // The graph's log axis, 16..250 Hz
    static double subharmX(double hz);

signals:
    void toolsChanged();
    void headroomChanged();
    void subharmMeterChanged();
    void tubeGlowChanged();
    void upmixStatusChanged();
    void unprotectedChanged();   // -- WP P1QT2 --

private:
    void apply(const QString &what, std::function<bool(FfiCore *)> setter, std::function<void(bool)> done = {});
    void limiterWrite(const QString &what, std::function<bool(FfiCore *)> setter);
    bool limiterOutput(int output) const;
    void fetchHeadroom();
    void updatePolls();
    void updateGlow();
    void readUnprotected();   // -- WP P1QT2 --

    AppContext m_ctx;
    int m_reads = 0;
    double m_headroom = 0;
    QVariantList m_subMeter;
    double m_glow = 0;
    QVariantMap m_upmixStatus;
    bool m_subharmShown = false;
    bool m_tubeShown = false;
    bool m_upmixShown = false;
    Poller::Id m_subMeterPoll = -1;
    Poller::Id m_upmixPoll = -1;
    QVariantList m_unprotected;        // -- WP P1QT2 --
    bool m_unprotectedAsked = false;   // -- WP P1QT2 --
};

#endif // TOOLSCONTROLLER_H
