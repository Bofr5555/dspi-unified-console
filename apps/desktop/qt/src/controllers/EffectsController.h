#ifndef EFFECTSCONTROLLER_H
#define EFFECTSCONTROLLER_H

#include <QObject>
#include <QString>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "CoreApi.h"
#include "Registrar.h"

class QJSEngine;

// -- WP QP7 -- The four effect windows' controller (the Mac's CrossfeedView,
// LoudnessView, VolumeLevellerView, PsychoacousticBassView and their
// ToolParameters): what the device's state says about crossfeed, loudness,
// the leveller and psychoacoustic bass, the writes, and the windows' curves.
//
// Reads come from DeviceModel's snapshot. Every write goes through apply(),
// keyed by its target ("crossfeed freq"), so a slider's live values and its
// release replace each other in the queue, newest first. The master switches
// use SidebarController's keys ("crossfeed enabled"), so the quick strip and a
// window's switch coalesce too.
//
// The curves are the Mac's drawings, for the eye only: the BS2B crossfeed
// filter's direct and opposite paths at 48 kHz, ISO 226:2003 compensation at
// -40 dB, and the psychoacoustic-bass sketch. Graph maps carry x as a 0..1
// position on the 20 Hz..20 kHz log axis and y as 0..1 from the bottom.
class EffectsController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(int revision READ revision NOTIFY effectsChanged)
    Q_PROPERTY(bool connected READ connected NOTIFY effectsChanged)
    Q_PROPERTY(bool writable READ writable NOTIFY effectsChanged)
    Q_PROPERTY(int numOutputs READ numOutputs NOTIFY effectsChanged)
    Q_PROPERTY(int pdmOutput READ pdmOutput NOTIFY effectsChanged)

    // Crossfeed. freq and feed are the device's custom values; shownFreq and
    // shownFeed are what the rows show: the preset's own while one is chosen.
    Q_PROPERTY(bool crossfeedOn READ crossfeedOn NOTIFY effectsChanged)
    Q_PROPERTY(int crossfeedPreset READ crossfeedPreset NOTIFY effectsChanged)
    Q_PROPERTY(double crossfeedFreq READ crossfeedFreq NOTIFY effectsChanged)
    Q_PROPERTY(double crossfeedFeed READ crossfeedFeed NOTIFY effectsChanged)
    Q_PROPERTY(double crossfeedShownFreq READ crossfeedShownFreq NOTIFY effectsChanged)
    Q_PROPERTY(double crossfeedShownFeed READ crossfeedShownFeed NOTIFY effectsChanged)
    Q_PROPERTY(bool crossfeedItd READ crossfeedItd NOTIFY effectsChanged)
    Q_PROPERTY(int crossfeedPairMask READ crossfeedPairMask NOTIFY effectsChanged)
    Q_PROPERTY(bool crossfeedMaskSupported READ crossfeedMaskSupported NOTIFY effectsChanged)
    Q_PROPERTY(int crossfeedPairCount READ crossfeedPairCount NOTIFY effectsChanged)
    Q_PROPERTY(QVariantList crossfeedPresets READ crossfeedPresets CONSTANT)
    Q_PROPERTY(int crossfeedCustomPreset READ crossfeedCustomPreset CONSTANT)

    // Loudness
    Q_PROPERTY(bool loudnessOn READ loudnessOn NOTIFY effectsChanged)
    Q_PROPERTY(double loudnessRef READ loudnessRef NOTIFY effectsChanged)
    Q_PROPERTY(double loudnessIntensity READ loudnessIntensity NOTIFY effectsChanged)
    Q_PROPERTY(int loudnessMask READ loudnessMask NOTIFY effectsChanged)
    Q_PROPERTY(bool loudnessMaskSupported READ loudnessMaskSupported NOTIFY effectsChanged)

    // Leveller. The channel masks are inputs; the section shows only with
    // more than two inputs, as on the Mac.
    Q_PROPERTY(bool levellerOn READ levellerOn NOTIFY effectsChanged)
    Q_PROPERTY(double levellerAmount READ levellerAmount NOTIFY effectsChanged)
    Q_PROPERTY(int levellerSpeed READ levellerSpeed NOTIFY effectsChanged)
    Q_PROPERTY(double levellerMaxGain READ levellerMaxGain NOTIFY effectsChanged)
    Q_PROPERTY(bool levellerLookahead READ levellerLookahead NOTIFY effectsChanged)
    Q_PROPERTY(double levellerGate READ levellerGate NOTIFY effectsChanged)
    Q_PROPERTY(int levellerDetectorMask READ levellerDetectorMask NOTIFY effectsChanged)
    Q_PROPERTY(int levellerApplyMask READ levellerApplyMask NOTIFY effectsChanged)
    Q_PROPERTY(int levellerChannels READ levellerChannels NOTIFY effectsChanged)
    Q_PROPERTY(bool levellerMasksShown READ levellerMasksShown NOTIFY effectsChanged)

    // Psychoacoustic bass
    Q_PROPERTY(bool psybassSupported READ psybassSupported NOTIFY effectsChanged)
    Q_PROPERTY(bool psybassOn READ psybassOn NOTIFY effectsChanged)
    Q_PROPERTY(double psybassCutoff READ psybassCutoff NOTIFY effectsChanged)
    Q_PROPERTY(double psybassHarmonics READ psybassHarmonics NOTIFY effectsChanged)
    Q_PROPERTY(double psybassDrive READ psybassDrive NOTIFY effectsChanged)
    Q_PROPERTY(double psybassCharacter READ psybassCharacter NOTIFY effectsChanged)
    Q_PROPERTY(double psybassOriginal READ psybassOriginal NOTIFY effectsChanged)
    Q_PROPERTY(int psybassMask READ psybassMask NOTIFY effectsChanged)
    Q_PROPERTY(QVariantList psybassStartingPoints READ psybassStartingPoints CONSTANT)
public:
    EffectsController(const AppContext &ctx, QObject *parent);
    static EffectsController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<EffectsController>(); }

    int revision() const { return m_reads; }
    bool connected() const;
    bool writable() const;
    int numOutputs() const;
    int pdmOutput() const { return numOutputs() - 1; }

    bool crossfeedOn() const;
    int crossfeedPreset() const;
    double crossfeedFreq() const;
    double crossfeedFeed() const;
    double crossfeedShownFreq() const;
    double crossfeedShownFeed() const;
    bool crossfeedItd() const;
    int crossfeedPairMask() const;
    bool crossfeedMaskSupported() const;
    int crossfeedPairCount() const;
    static QVariantList crossfeedPresets();
    static int crossfeedCustomPreset() { return 3; }

    bool loudnessOn() const;
    double loudnessRef() const;
    double loudnessIntensity() const;
    int loudnessMask() const;
    bool loudnessMaskSupported() const;

    bool levellerOn() const;
    double levellerAmount() const;
    int levellerSpeed() const;
    double levellerMaxGain() const;
    bool levellerLookahead() const;
    double levellerGate() const;
    int levellerDetectorMask() const;
    int levellerApplyMask() const;
    int levellerChannels() const;
    bool levellerMasksShown() const;

    bool psybassSupported() const;
    bool psybassOn() const;
    double psybassCutoff() const;
    double psybassHarmonics() const;
    double psybassDrive() const;
    double psybassCharacter() const;
    double psybassOriginal() const;
    int psybassMask() const;
    static QVariantList psybassStartingPoints();

    // ---- Writes. Values are clamped to the Mac's ranges first. ----
    // `effect`: "crossfeed", "loudness", "leveller", "psybass"
    Q_INVOKABLE void setEnabled(const QString &effect, bool on);

    Q_INVOKABLE void setCrossfeedPreset(int preset);
    // A drag's value (`live`) sends the number only; the release, or a typed
    // value, also makes the preset Custom when it is not, as the Mac does.
    Q_INVOKABLE void setCrossfeedFreq(double hz, bool live = false);
    Q_INVOKABLE void setCrossfeedFeed(double db, bool live = false);
    Q_INVOKABLE void setCrossfeedItd(bool on);
    Q_INVOKABLE void setCrossfeedPairMask(int mask);
    Q_INVOKABLE void toggleCrossfeedPair(int pair);
    // Bits for the "All pairs" row
    Q_INVOKABLE int allPairsMask() const;

    Q_INVOKABLE void setLoudnessRef(double spl);
    Q_INVOKABLE void setLoudnessIntensity(double pct);
    Q_INVOKABLE void setLoudnessMask(int mask);
    Q_INVOKABLE void toggleLoudnessOutput(int output);

    Q_INVOKABLE void setLevellerAmount(double pct);
    Q_INVOKABLE void setLevellerSpeed(int speedIndex);
    Q_INVOKABLE void setLevellerMaxGain(double db);
    Q_INVOKABLE void setLevellerLookahead(bool on);
    Q_INVOKABLE void setLevellerGate(double db);
    Q_INVOKABLE void setLevellerMasks(int detectorMask, int applyMask);
    Q_INVOKABLE void toggleLevellerChannel(bool detector, int channel);

    // `param`: "cutoff", "harmonics", "drive", "character", "original"
    Q_INVOKABLE void setPsybass(const QString &param, double value);
    Q_INVOKABLE void applyPsybassStartingPoint(int index);
    Q_INVOKABLE void setPsybassMask(int mask);
    Q_INVOKABLE void togglePsybassOutput(int output);
    Q_INVOKABLE int allOutputsMask() const;
    // Every output but the PDM sub ("Exclude sub (recommended)")
    Q_INVOKABLE int outputsWithoutSubMask() const;

    // ---- Names for the chips' tooltips ----
    Q_INVOKABLE QString outputName(int output) const;
    Q_INVOKABLE QString inputName(int input) const;

    // ---- Curves ----
    // {direct: [x, y, ...], crossfeed: [x, y, ...], gridDb: [...], gridY: [...]}
    Q_INVOKABLE QVariantMap crossfeedGraph(double freqHz, double feedDb) const;
    // {curve: [x, y, ...], gridDb: [...], gridY: [...]}
    Q_INVOKABLE QVariantMap loudnessGraph(double refSpl, double intensityPct) const;
    // {fc, fc4: x positions; original, harmonics: band heights 0..1}
    Q_INVOKABLE QVariantMap psybassSketch(double cutoffHz, double harmonicsDb, double originalDb) const;
    // The x position of `hz` on the graphs' log axis
    Q_INVOKABLE static double freqX(double hz);

    // The Mac's formulas, for the tests
    static double crossfeedDirectDb(double fc, double feedDb, double f);
    static double crossfeedOppositeDb(double fc, double feedDb, double f);
    static double loudnessCompensationDb(double refSpl, double intensityPct, int isoRow);
    static int isoRows();
    static double isoFreq(int row);

signals:
    void effectsChanged();

private:
    void apply(const QString &what, std::function<bool(FfiCore *)> setter);
    void setPsybassParam(const QString &key, uint8_t param, double value);
    void setMask16(const QString &what, int mask, bool (*setter)(FfiCore *, uint16_t));

    AppContext m_ctx;
    int m_reads = 0;
};

#endif // EFFECTSCONTROLLER_H
