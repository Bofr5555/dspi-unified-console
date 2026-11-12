#ifndef CHANNELCONTROLLER_H
#define CHANNELCONTROLLER_H

#include <QHash>
#include <QObject>
#include <QString>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>

#include "CoreApi.h"
#include "Poller.h"
#include "Registrar.h"

// -- WP QP2 -- The channel pages' controller: the input header (preamp, pair
// link, clear), the filter lists (PEQ and crossover bands), the output card
// (routing of the first inputs, gain, delay, mute, enable, limiter).
//
// Reads come from DeviceModel's snapshot; every write goes through one place,
// apply(), which calls DeviceModel::write() keyed by its target. A band edit is
// staged (band() returns it at once) until its write's snapshot arrives, so
// two quick edits of one band never lose the first.
//
// Wire channels: inputs are 0..n-1, output o is channel n + o, where n is the
// state's input count (as ChannelListModel numbers them).
//
// What the firmware takes decides what is offered: availableTypes() follows
// the Mac's availableFilterTypes (firmware 1.1.4 for notch, all-pass and band
// bypass; wire V13/V14/V28 for the first-order types; V22 for the Linkwitz
// Transform, outputs only), crossoverSupported is wire V11, and the limiter
// cell needs the core's WIRE_FEAT_LIMITER.
class ChannelController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // Bumped on every applied snapshot and every staged edit: QML bindings
    // that call band()/output() name it so they re-read
    Q_PROPERTY(int revision READ revision NOTIFY changed)
    Q_PROPERTY(bool writable READ writable NOTIFY changed)
    Q_PROPERTY(bool bypassSupported READ bypassSupported NOTIFY changed)
    Q_PROPERTY(bool crossoverSupported READ crossoverSupported NOTIFY changed)
    Q_PROPERTY(bool limiterSupported READ limiterSupported NOTIFY changed)
    Q_PROPERTY(int numInputs READ numInputs NOTIFY changed)
    Q_PROPERTY(int numOutputs READ numOutputs NOTIFY changed)
    Q_PROPERTY(int pdmOutput READ pdmOutput NOTIFY changed)
    Q_PROPERTY(double maxDelayMs READ maxDelayMs NOTIFY limitsChanged)
    Q_PROPERTY(double preampMaxDb READ preampMaxDb NOTIFY limitsChanged)
    Q_PROPERTY(int linkedPairs READ linkedPairs NOTIFY changed)
    // An output page shows its XO tab (the graph draws crossover bands then)
    Q_PROPERTY(bool crossoverTabShown READ crossoverTabShown WRITE setCrossoverTabShown NOTIFY crossoverTabShownChanged)
    // Each output's limiter gain reduction in dB, polled while a page shows it
    Q_PROPERTY(QVariantList limiterReduction READ limiterReduction NOTIFY limiterReductionChanged)
public:
    ChannelController(const AppContext &ctx, QObject *parent);
    static ChannelController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<ChannelController>(); }

    // Bands per list: PEQ and crossover
    static constexpr int PeqBands = BANDS_PER_CHANNEL;
    static constexpr int XoBands = MAX_XOVER_BANDS;

    int revision() const { return m_revision; }
    bool writable() const;
    bool bypassSupported() const;
    bool crossoverSupported() const;
    bool limiterSupported() const;
    int numInputs() const;
    int numOutputs() const;
    int pdmOutput() const { return numOutputs() - 1; }
    double maxDelayMs() const { return m_maxDelayMs; }
    double preampMaxDb() const { return m_preampMaxDb; }
    int linkedPairs() const { return m_linkedPairs; }
    bool crossoverTabShown() const { return m_xoTab; }
    void setCrossoverTabShown(bool shown);
    QVariantList limiterReduction() const { return m_reduction; }

    // ---- Filter types ----------------------------------------------------
    // The PEQ types this firmware takes, in the Mac's menu order
    Q_INVOKABLE QVariantList availableTypes(bool includeLinkwitz) const;
    // The type picker's menu: [{name, variants: [{type, label}]}], a group with
    // one variant is a plain row, more make a submenu (Low Shelf > 6 dB/oct ...)
    Q_INVOKABLE QVariantList typeMenu(bool includeLinkwitz) const;
    // The core's name for a type ("High Cut (12dB)")
    Q_INVOKABLE QString typeName(int type) const;
    Q_INVOKABLE bool usesGain(int type) const;
    Q_INVOKABLE bool usesQ(int type) const;
    // Crossover types: family 0 Off, 1 Linkwitz-Riley, 2 Butterworth, 3 Bessel
    Q_INVOKABLE QVariantList crossoverFamilies() const;
    Q_INVOKABLE int familyOf(int type) const;
    Q_INVOKABLE int orderOf(int type) const;
    Q_INVOKABLE bool isLowPass(int type) const;
    Q_INVOKABLE QVariantList ordersOf(int family) const;
    // The crossover type of a family, order and direction; -1 if none
    Q_INVOKABLE static int crossoverType(int family, int order, bool lowPass);
    // The Linkwitz Transform's implied DC boost, 40 log10(f0/fp) dB
    Q_INVOKABLE static double linkwitzBoost(double f0, double fp);

    // ---- Bands -----------------------------------------------------------
    // One band as shown: {type, freq, q, gain, qp, bypass}; `crossover` reads
    // crossover band 0..3, otherwise PEQ band 0..9. A staged edit wins.
    Q_INVOKABLE QVariantMap band(int channel, int band, bool crossover) const;
    // Edits of one band; a linked input pair gets the same edit on both halves
    Q_INVOKABLE void setBandType(int channel, int band, bool crossover, int type);
    Q_INVOKABLE void setBandValue(int channel, int band, bool crossover, const QString &field, double value);
    Q_INVOKABLE void setBandBypass(int channel, int band, bool crossover, bool bypass);
    // The Linkwitz panel's Apply: the whole transform at once
    Q_INVOKABLE void setLinkwitz(int channel, int band, double f0, double q0, double fp, double qp);
    // Crossover row pickers; each keeps the other two axes where it can
    Q_INVOKABLE void setCrossoverFamily(int channel, int band, int family);
    Q_INVOKABLE void setCrossoverLowPass(int channel, int band, bool lowPass);
    Q_INVOKABLE void setCrossoverOrder(int channel, int band, int order);
    // Enable All / Bypass All (bands that are Off are left alone)
    Q_INVOKABLE void setAllBypassed(int channel, bool crossover, bool bypass);
    // Every band of the list back to Off, 1000 Hz, Q 0.707, 0 dB
    Q_INVOKABLE void clearBands(int channel, bool crossover);

    // ---- Inputs ----------------------------------------------------------
    Q_INVOKABLE double preamp(int input) const;
    // Sent live during a drag and again on release; mirrored onto a linked partner
    Q_INVOKABLE void setPreamp(int input, double db);
    // Pair links (inputs 2p and 2p+1), kept per device serial
    Q_INVOKABLE bool pairAvailable(int input) const;
    Q_INVOKABLE bool isLinked(int input) const;
    Q_INVOKABLE int linkedPartner(int input) const;
    // {bands, preamp}: what differs between the two halves of the pair
    Q_INVOKABLE QVariantMap pairMismatch(int input) const;
    Q_INVOKABLE void unlinkPair(int input);
    // Links the pair, first copying `keep`'s bands and trim onto the other
    // half (-1: they already match)
    Q_INVOKABLE void linkPair(int input, int keep);
    // Clear PEQ: this input, and its partner when linked
    Q_INVOKABLE void clearInputPeq(int input);

    // ---- Outputs ---------------------------------------------------------
    Q_INVOKABLE int outputChannel(int output) const;
    // {enabled, muted, gain, delay, limiter, limiterThreshold}
    Q_INVOKABLE QVariantMap output(int output) const;
    Q_INVOKABLE void setOutputGain(int output, double db);
    // A delay as given (typed, file); the slider rounds its own values (SL1)
    Q_INVOKABLE void setOutputDelay(int output, double ms);
    Q_INVOKABLE void setOutputMute(int output, bool muted);
    // Enabling may need a question first (PDM and the Core 1 EQ outputs share a
    // core): then core1Question() is emitted and nothing is sent until
    // confirmOutputEnable(). Returns false when it asked.
    Q_INVOKABLE bool requestOutputEnable(int output, bool enabled);
    Q_INVOKABLE bool enableWouldConflict(int output) const;
    Q_INVOKABLE void confirmOutputEnable(int output);
    Q_INVOKABLE void setLimiterEnabled(int output, bool enabled);
    // The limiter cell's right-click: emits limiterSettingsRequested for the
    // limiter's own window (not part of this page)
    Q_INVOKABLE void requestLimiterSettings(int output);
    // The routing rows: {enabled, gain, invert} of input -> output
    Q_INVOKABLE QVariantMap route(int input, int output) const;
    Q_INVOKABLE void setRoute(int input, int output, bool enabled, double gain, bool invert);
    // An output page counts itself while shown; the limiter meter polls meanwhile
    Q_INVOKABLE void watchLimiter(bool on);

signals:
    void changed();
    void limitsChanged();
    void crossoverTabShownChanged();
    void limiterReductionChanged();
    // The question before an enable that switches the other side of Core 1 off
    void core1Question(int output, const QString &title, const QString &text, const QString &acceptText);
    void limiterSettingsRequested(int output);

private:
    struct Staged {
        FilterParams params;
        quint64 seq = 0;
    };
    static int stageKey(int channel, int band, bool crossover) { return channel * 64 + (crossover ? 32 : 0) + band; }
    bool validBand(int channel, int band, bool crossover) const;
    FilterParams current(int channel, int band, bool crossover) const;
    // The one write path: `what` names the target (it is the coalescing key)
    void apply(const QString &what, std::function<bool(FfiCore *)> setter);
    void writeBand(int channel, int band, bool crossover, const FilterParams &p);
    // The band edit on `channel` and, for half of a linked input pair, its partner
    void editBand(int channel, int band, bool crossover, const std::function<void(FilterParams &)> &edit);
    void onSnapshot();
    void loadLinks();
    void storeLinks();
    std::tuple<int, int, int> firmware() const;
    int wireFormat() const;

    AppContext m_ctx;
    QHash<int, Staged> m_staged;
    quint64 m_seq = 0;
    int m_revision = 0;
    int m_linkedPairs = 1;
    QString m_linksSerial;
    uint64_t m_generation = 0;
    double m_maxDelayMs = 21;
    double m_preampMaxDb = 10;
    bool m_xoTab = false;
    int m_limiterWatchers = 0;
    Poller::Id m_limiterPoll = 0;
    QVariantList m_reduction;
    QHash<int, QString> m_typeNames;
};

#endif // CHANNELCONTROLLER_H
