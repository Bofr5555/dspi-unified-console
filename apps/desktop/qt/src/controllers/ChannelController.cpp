// -- WP QP2 -- The channel pages' controller (ChannelController.h).
#include "ChannelController.h"

#include "Core.h"
#include "DeviceModel.h"
#include "HardwareController.h" // -- WP QPI2 --
#include "Snapshot.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QSettings>
#include <array>
#include <cmath>

static const Registrar::Controller<ChannelController> channelController;

namespace {

// The bands Clear writes (the Mac's FilterParams default)
FilterParams flatBand() {
    FilterParams p{};
    p.filter_type = FILTER_FLAT;
    p.freq = 1000.0f;
    p.q = 0.707f;
    p.gain = 0.0f;
    p.qp = 0.707f;
    return p;
}

// Crossover families as the XO row's FAMILY picker numbers them
enum Family { Off = 0, LinkwitzRiley = 1, Butterworth = 2, Bessel = 3 };

// The type codes 32..63 in blocks per family: Linkwitz-Riley orders 2, 4, 6,
// 8 at 32; Butterworth 1..8 at 40; Bessel 2, 4, 6, 8 at 56; low pass even,
// high pass odd (the firmware's crossover type contract).
int familyBase(int family) {
    switch (family) {
    case LinkwitzRiley: return 32;
    case Butterworth: return 40;
    case Bessel: return 56;
    default: return -1;
    }
}

bool firstOrderPeq(int type) {
    return type == FILTER_ALLPASS1 || type == FILTER_LOWSHELF1 || type == FILTER_HIGHSHELF1 ||
           type == FILTER_LOWPASS1 || type == FILTER_HIGHPASS1;
}

bool isCrossover(int type) { return type >= FILTER_XOVER_FIRST && type <= FILTER_XOVER_LAST; }

} // namespace

ChannelController::ChannelController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    // The type names are the core's table (no device involved, nothing blocks)
    const QJsonValue names = jsonValue(readSized([](char *buf, uint32_t len) { return dspi_filter_type_names(buf, len); }));
    for (const QJsonValue &v : names.toArray()) {
        const QJsonObject o = v.toObject();
        m_typeNames.insert(o.value(QLatin1String("code")).toInt(), o.value(QLatin1String("name")).toString());
    }
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &ChannelController::onSnapshot);
    m_limiterPoll = ctx.poller->add(
        QStringLiteral("output limiter meter"), 64, this,
        [](FfiCore *c) {
            std::array<float, MAX_OUTPUTS> gr{};
            const bool ok = dspi_fetch_limiter_meter(c, gr.data());
            QVariantList out;
            if (ok)
                for (float v : gr) out.append(double(v));
            return out;
        },
        [this](const QVariantList &gr) {
            if (gr.isEmpty() || gr == m_reduction) return;
            m_reduction = gr;
            emit limiterReductionChanged();
        });
    ctx.poller->setEnabled(m_limiterPoll, false);
    onSnapshot();
}

// ---- State ---------------------------------------------------------------

bool ChannelController::writable() const { return m_ctx.device->connected() && !m_ctx.device->readOnly(); }

std::tuple<int, int, int> ChannelController::firmware() const {
    const DspState *s = m_ctx.device->state();
    if (!s) return { 0, 0, 0 };
    return { s->fw_major, s->fw_minor, s->fw_patch };
}

int ChannelController::wireFormat() const { return m_ctx.device->wireFormat(); }

// Per-band bypass, notch and the two-pole all-pass shipped in firmware 1.1.4
bool ChannelController::bypassSupported() const { return firmware() >= std::make_tuple(1, 1, 4); }
bool ChannelController::crossoverSupported() const { return wireFormat() >= 11; }
bool ChannelController::limiterSupported() const {
    return (m_ctx.device->snapshot().wireFeatures & WIRE_FEAT_LIMITER) != 0;
}

int ChannelController::numInputs() const {
    const DspState *s = m_ctx.device->state();
    return s ? qBound(0, int(s->num_input_channels), MAX_INPUTS) : 0;
}

int ChannelController::numOutputs() const { return m_ctx.device->numOutputs(); }

void ChannelController::setCrossoverTabShown(bool shown) {
    if (shown == m_xoTab) return;
    m_xoTab = shown;
    emit crossoverTabShownChanged();
}

void ChannelController::onSnapshot() {
    const DeviceSnapshot &snap = m_ctx.device->snapshot();
    const QString serial = m_ctx.device->serial();
    if (!serial.isEmpty() && serial != m_linksSerial) {
        m_linksSerial = serial;
        loadLinks();
    }
    if (!snap.connected) {
        m_staged.clear();
        m_generation = 0;
    } else if (snap.generation != m_generation) {
        // A device (re)opened: its limits, read once on the worker
        m_generation = snap.generation;
        m_staged.clear();
        m_ctx.core->worker()->call(
            this,
            [](FfiCore *c) { return std::make_pair(double(dspi_max_delay_ms(c)), double(dspi_preamp_max_db(c))); },
            [this](std::pair<double, double> limits) {
                if (limits.first > 0) m_maxDelayMs = limits.first;
                if (limits.second > 0) m_preampMaxDb = limits.second;
                emit limitsChanged();
            });
    }
    m_ctx.poller->setEnabled(m_limiterPoll, m_limiterWatchers > 0 && limiterSupported());
    ++m_revision;
    emit changed();
}

void ChannelController::watchLimiter(bool on) {
    m_limiterWatchers = qMax(0, m_limiterWatchers + (on ? 1 : -1));
    m_ctx.poller->setEnabled(m_limiterPoll, m_limiterWatchers > 0 && limiterSupported());
}

void ChannelController::apply(const QString &what, std::function<bool(FfiCore *)> setter, std::function<void(bool)> done) {
    m_ctx.device->write(what, std::move(setter), std::move(done));
}

// ---- Filter types ------------------------------------------------------------

QVariantList ChannelController::availableTypes(bool includeLinkwitz) const {
    const int wire = wireFormat();
    QVariantList out{ FILTER_FLAT, FILTER_PEAKING, FILTER_LOWSHELF, FILTER_HIGHSHELF, FILTER_LOWPASS, FILTER_HIGHPASS };
    if (bypassSupported()) out << FILTER_NOTCH << FILTER_ALLPASS;
    if (wire >= 13) out << FILTER_ALLPASS1;
    if (wire >= 14) out << FILTER_LOWSHELF1 << FILTER_HIGHSHELF1;
    if (wire >= 28) out << FILTER_LOWPASS1 << FILTER_HIGHPASS1;
    if (includeLinkwitz && wire >= 22) out << FILTER_LINKWITZ_TRANSFORM;
    return out;
}

QVariantList ChannelController::typeMenu(bool includeLinkwitz) const {
    // The Mac's PEQ menu: a shape, then its orders gentler first
    struct Group {
        const char *name;
        std::initializer_list<int> variants;
    };
    static const Group groups[] = {
        { QT_TR_NOOP("Off"), { FILTER_FLAT } },
        { QT_TR_NOOP("Peaking"), { FILTER_PEAKING } },
        { QT_TR_NOOP("Low Shelf"), { FILTER_LOWSHELF1, FILTER_LOWSHELF } },
        { QT_TR_NOOP("High Shelf"), { FILTER_HIGHSHELF1, FILTER_HIGHSHELF } },
        { QT_TR_NOOP("High Cut"), { FILTER_LOWPASS1, FILTER_LOWPASS } },
        { QT_TR_NOOP("Low Cut"), { FILTER_HIGHPASS1, FILTER_HIGHPASS } },
        { QT_TR_NOOP("Notch"), { FILTER_NOTCH } },
        { QT_TR_NOOP("All Pass"), { FILTER_ALLPASS1, FILTER_ALLPASS } },
        { QT_TR_NOOP("Linkwitz Transform"), { FILTER_LINKWITZ_TRANSFORM } },
    };
    const QVariantList allowed = availableTypes(includeLinkwitz);
    QVariantList menu;
    for (const Group &g : groups) {
        QVariantList variants;
        for (int t : g.variants) {
            if (!allowed.contains(t)) continue;
            QString label;
            switch (t) {
            case FILTER_LOWSHELF1: case FILTER_HIGHSHELF1: case FILTER_LOWPASS1: case FILTER_HIGHPASS1:
                label = tr("6 dB/oct"); break;
            case FILTER_LOWSHELF: case FILTER_HIGHSHELF: case FILTER_LOWPASS: case FILTER_HIGHPASS:
                label = tr("12 dB/oct"); break;
            case FILTER_ALLPASS1: label = tr("180°"); break;
            case FILTER_ALLPASS: label = tr("360°"); break;
            default: label = typeName(t);
            }
            variants.append(QVariantMap{ { QStringLiteral("type"), t }, { QStringLiteral("label"), label } });
        }
        if (variants.isEmpty()) continue;
        menu.append(QVariantMap{ { QStringLiteral("name"), tr(g.name) }, { QStringLiteral("variants"), variants } });
    }
    return menu;
}

QString ChannelController::typeName(int type) const {
    const QString name = m_typeNames.value(type);
    return name.isEmpty() ? tr("Type %1").arg(type) : name;
}

// A dB gain to edit: the Linkwitz Transform's gain field is fp in Hz
bool ChannelController::usesGain(int type) const {
    return type == FILTER_PEAKING || type == FILTER_LOWSHELF || type == FILTER_HIGHSHELF ||
           type == FILTER_LOWSHELF1 || type == FILTER_HIGHSHELF1;
}

// Q to edit: the firmware ignores it on first-order and crossover sections
bool ChannelController::usesQ(int type) const {
    if (type == FILTER_FLAT || type == FILTER_LINKWITZ_TRANSFORM) return false;
    return !isCrossover(type) && !firstOrderPeq(type);
}

QVariantList ChannelController::crossoverFamilies() const {
    return { tr("Off"), tr("Linkwitz-Riley"), tr("Butterworth"), tr("Bessel") };
}

int ChannelController::familyOf(int type) const {
    if (type >= 32 && type <= 39) return LinkwitzRiley;
    if (type >= 40 && type <= 55) return Butterworth;
    if (type >= 56 && type <= 63) return Bessel;
    return Off;
}

int ChannelController::orderOf(int type) const {
    switch (familyOf(type)) {
    case LinkwitzRiley: return ((type - 32) / 2 + 1) * 2;
    case Butterworth: return (type - 40) / 2 + 1;
    case Bessel: return ((type - 56) / 2 + 1) * 2;
    default: return 0;
    }
}

bool ChannelController::isLowPass(int type) const { return !isCrossover(type) || type % 2 == 0; }

QVariantList ChannelController::ordersOf(int family) const {
    if (family == Butterworth) return { 1, 2, 3, 4, 5, 6, 7, 8 };
    if (family == LinkwitzRiley || family == Bessel) return { 2, 4, 6, 8 };
    return {};
}

int ChannelController::crossoverType(int family, int order, bool lowPass) {
    const int base = familyBase(family);
    if (base < 0) return -1;
    int step;
    if (family == Butterworth) {
        if (order < 1 || order > 8) return -1;
        step = order - 1;
    } else {
        if (order < 2 || order > 8 || order % 2) return -1;
        step = order / 2 - 1;
    }
    return base + step * 2 + (lowPass ? 0 : 1);
}

double ChannelController::linkwitzBoost(double f0, double fp) {
    if (!(fp > 0) || !(f0 > 0)) return 0;
    return 40.0 * std::log10(f0 / fp);
}

// ---- Bands ---------------------------------------------------------------

bool ChannelController::validBand(int channel, int band, bool crossover) const {
    if (channel < 0 || channel >= MAX_CHANNELS || band < 0) return false;
    if (crossover) return band < XoBands && channel >= numInputs();
    return band < PeqBands;
}

FilterParams ChannelController::current(int channel, int band, bool crossover) const {
    const auto staged = m_staged.constFind(stageKey(channel, band, crossover));
    if (staged != m_staged.cend()) return staged->params;
    const DspState *s = m_ctx.device->state();
    if (!s || !validBand(channel, band, crossover)) return flatBand();
    return crossover ? s->xover[channel][band] : s->filters[channel][band];
}

QVariantMap ChannelController::band(int channel, int band, bool crossover) const {
    const FilterParams p = current(channel, band, crossover);
    return { { QStringLiteral("type"), int(p.filter_type) }, { QStringLiteral("freq"), double(p.freq) },
             { QStringLiteral("q"), double(p.q) },            { QStringLiteral("gain"), double(p.gain) },
             { QStringLiteral("qp"), double(p.qp) },          { QStringLiteral("bypass"), p.bypass } };
}

void ChannelController::writeBand(int channel, int band, bool crossover, const FilterParams &p) {
    if (!writable() || !validBand(channel, band, crossover)) return;
    const int key = stageKey(channel, band, crossover);
    const quint64 seq = ++m_seq;
    m_staged.insert(key, { p, seq });
    ++m_revision;
    emit changed();
    const auto ch = uint8_t(channel), b = uint8_t(band);
    const QString what = QStringLiteral("%1 band %2 %3").arg(crossover ? QStringLiteral("crossover") : QStringLiteral("filter")).arg(channel).arg(band);
    apply(
        what,
        [ch, b, crossover, p](FfiCore *c) { return crossover ? dspi_set_crossover(c, ch, b, p) : dspi_set_filter(c, ch, b, p); },
        [this, key, seq](bool) {
            // The snapshot that came with it is the truth now, unless a newer edit is staged
            const auto it = m_staged.constFind(key);
            if (it != m_staged.cend() && it->seq == seq) {
                m_staged.remove(key);
                ++m_revision;
                emit changed();
            }
        });
}

void ChannelController::editBand(int channel, int band, bool crossover, const std::function<void(FilterParams &)> &edit) {
    if (!validBand(channel, band, crossover)) return;
    FilterParams p = current(channel, band, crossover);
    edit(p);
    writeBand(channel, band, crossover, p);
    // Inputs only: crossovers are output bands
    const int partner = crossover ? -1 : linkedPartner(channel);
    if (partner >= 0) writeBand(partner, band, false, p);
}

void ChannelController::setBandType(int channel, int band, bool crossover, int type) {
    editBand(channel, band, crossover, [type](FilterParams &p) {
        const bool wasLt = p.filter_type == FILTER_LINKWITZ_TRANSFORM;
        const bool isLt = type == FILTER_LINKWITZ_TRANSFORM;
        p.filter_type = uint8_t(type);
        // The Linkwitz Transform reads gain as fp in Hz: entering it starts a
        // flat transform (target = driver), leaving it drops fp back to 0 dB
        if (isLt && !wasLt) {
            p.freq = qMax(p.freq, 10.0f);
            p.q = qBound(0.1f, p.q, 20.0f);
            p.gain = p.freq;
            p.qp = p.q;
        } else if (!isLt && wasLt) {
            p.gain = 0;
        }
    });
}

void ChannelController::setBandValue(int channel, int band, bool crossover, const QString &field, double value) {
    if (!std::isfinite(value)) return;
    const QString f = field;
    editBand(channel, band, crossover, [f, value, crossover](FilterParams &p) {
        if (f == QLatin1String("freq")) p.freq = float(qMax(value, 10.0));
        else if (f == QLatin1String("gain") && !crossover) p.gain = float(value);
        else if (f == QLatin1String("q")) p.q = float(qBound(0.1, value, 20.0));
    });
}

void ChannelController::setBandBypass(int channel, int band, bool crossover, bool bypass) {
    if (!bypassSupported()) return;
    editBand(channel, band, crossover, [bypass](FilterParams &p) { p.bypass = bypass; });
}

void ChannelController::setLinkwitz(int channel, int band, double f0, double q0, double fp, double qp) {
    if (!std::isfinite(f0) || !std::isfinite(q0) || !std::isfinite(fp) || !std::isfinite(qp)) return;
    editBand(channel, band, false, [=](FilterParams &p) {
        p.filter_type = FILTER_LINKWITZ_TRANSFORM;
        p.freq = float(qMax(f0, 10.0));
        p.q = float(qBound(0.1, q0, 20.0));
        p.gain = float(qMax(fp, 10.0));
        p.qp = float(qBound(0.1, qp, 20.0));
    });
}

void ChannelController::setCrossoverFamily(int channel, int band, int family) {
    editBand(channel, band, true, [this, family](FilterParams &p) {
        if (family == Off) {
            p.filter_type = FILTER_FLAT;
            return;
        }
        // Keep the order if the family has it, else the nearest; keep LP/HP
        const int now = orderOf(p.filter_type);
        int order = 0;
        for (const QVariant &o : ordersOf(family))
            if (order == 0 || std::abs(o.toInt() - now) < std::abs(order - now)) order = o.toInt();
        const int t = crossoverType(family, order, isLowPass(p.filter_type));
        if (t >= 0) p.filter_type = uint8_t(t);
    });
}

void ChannelController::setCrossoverLowPass(int channel, int band, bool lowPass) {
    editBand(channel, band, true, [this, lowPass](FilterParams &p) {
        const int t = crossoverType(familyOf(p.filter_type), orderOf(p.filter_type), lowPass);
        if (t >= 0) p.filter_type = uint8_t(t);
    });
}

void ChannelController::setCrossoverOrder(int channel, int band, int order) {
    editBand(channel, band, true, [this, order](FilterParams &p) {
        const int t = crossoverType(familyOf(p.filter_type), order, isLowPass(p.filter_type));
        if (t >= 0) p.filter_type = uint8_t(t);
    });
}

void ChannelController::setAllBypassed(int channel, bool crossover, bool bypass) {
    if (!bypassSupported()) return;
    const int n = crossover ? XoBands : PeqBands;
    for (int b = 0; b < n; ++b) {
        const FilterParams p = current(channel, b, crossover);
        if (p.filter_type == FILTER_FLAT || p.bypass == bypass) continue;
        setBandBypass(channel, b, crossover, bypass);
    }
}

void ChannelController::clearBands(int channel, bool crossover) {
    const int n = crossover ? XoBands : PeqBands;
    for (int b = 0; b < n; ++b) writeBand(channel, b, crossover, flatBand());
}

// ---- Inputs ----------------------------------------------------------------

double ChannelController::preamp(int input) const {
    const DspState *s = m_ctx.device->state();
    return s && input >= 0 && input < MAX_INPUTS ? double(s->input_preamp_db[input]) : 0.0;
}

void ChannelController::setPreamp(int input, double db) {
    if (!writable() || input < 0 || input >= numInputs() || !std::isfinite(db)) return;
    const float v = float(qBound(-60.0, db, m_preampMaxDb));
    // One write per channel of a linked pair, never the all-inputs preamp
    for (int ch : { input, linkedPartner(input) }) {
        if (ch < 0) continue;
        const auto in = uint8_t(ch);
        apply(QStringLiteral("input preamp %1").arg(ch), [in, v](FfiCore *c) { return dspi_set_input_preamp(c, in, v); });
    }
}

bool ChannelController::pairAvailable(int input) const {
    return input >= 0 && qMax(input, input ^ 1) < numInputs();
}

bool ChannelController::isLinked(int input) const {
    const int pair = input / 2;
    return input >= 0 && pair < MAX_INPUTS / 2 && (m_linkedPairs & (1 << pair));
}

// Both halves must be live: fewer active inputs suspend a link, not forget it
int ChannelController::linkedPartner(int input) const {
    return isLinked(input) && pairAvailable(input) ? (input ^ 1) : -1;
}

QVariantMap ChannelController::pairMismatch(int input) const {
    bool bands = false;
    const int partner = input ^ 1;
    if (pairAvailable(input)) {
        for (int b = 0; b < PeqBands && !bands; ++b) {
            const FilterParams x = current(input, b, false), y = current(partner, b, false);
            bands = x.filter_type != y.filter_type || x.freq != y.freq || x.q != y.q || x.gain != y.gain ||
                    x.bypass != y.bypass || (x.filter_type == FILTER_LINKWITZ_TRANSFORM && x.qp != y.qp);
        }
    }
    // Trims are kept to 0.1 dB, so anything finer is noise
    const bool trims = pairAvailable(input) && std::abs(preamp(input) - preamp(partner)) > 0.05;
    return { { QStringLiteral("bands"), bands }, { QStringLiteral("preamp"), trims } };
}

void ChannelController::unlinkPair(int input) {
    if (input < 0 || input / 2 >= MAX_INPUTS / 2) return;
    m_linkedPairs &= ~(1 << (input / 2));
    storeLinks();
    ++m_revision;
    emit changed();
}

void ChannelController::linkPair(int input, int keep) {
    if (!pairAvailable(input)) return;
    const int other = keep == input ? (input ^ 1) : keep == (input ^ 1) ? input : -1;
    if (other >= 0) {
        // One-way copies, made before the link bit is set so nothing mirrors back
        for (int b = 0; b < PeqBands; ++b) writeBand(other, b, false, current(keep, b, false));
        if (writable()) {
            const float v = float(preamp(keep));
            const auto in = uint8_t(other);
            apply(QStringLiteral("input preamp %1").arg(other), [in, v](FfiCore *c) { return dspi_set_input_preamp(c, in, v); });
        }
    }
    m_linkedPairs |= 1 << (input / 2);
    storeLinks();
    ++m_revision;
    emit changed();
}

void ChannelController::clearInputPeq(int input) {
    clearBands(input, false);
    const int partner = linkedPartner(input);
    if (partner >= 0) clearBands(partner, false);
}

// Kept per device under inputLinks/<serial> (behaviour.md §21), one bit per
// pair. Pair 0 (inputs 1/2) is linked until the person says otherwise.
void ChannelController::loadLinks() {
    QSettings s;
    m_linkedPairs = s.value(QStringLiteral("inputLinks/%1").arg(m_linksSerial), 1).toInt() & 0xF;
}

void ChannelController::storeLinks() {
    if (m_linksSerial.isEmpty()) return;
    QSettings s;
    s.setValue(QStringLiteral("inputLinks/%1").arg(m_linksSerial), m_linkedPairs);
}

// ---- Outputs ---------------------------------------------------------------

int ChannelController::outputChannel(int output) const { return numInputs() + output; }

QVariantMap ChannelController::output(int output) const {
    const DspState *s = m_ctx.device->state();
    if (!s || output < 0 || output >= MAX_OUTPUTS) return {};
    return { { QStringLiteral("enabled"), s->output_enabled[output] },
             { QStringLiteral("muted"), s->output_muted[output] },
             { QStringLiteral("gain"), double(s->output_gain_db[output]) },
             { QStringLiteral("delay"), double(s->output_delay_ms[output]) },
             { QStringLiteral("limiter"), s->limiter_enabled[output] },
             { QStringLiteral("limiterThreshold"), double(s->limiter_threshold_db[output]) } };
}

void ChannelController::setOutputGain(int output, double db) {
    if (!writable() || output < 0 || output >= numOutputs() || !std::isfinite(db)) return;
    const auto o = uint8_t(output);
    const float v = float(qBound(-60.0, db, 20.0)); // -- WP QPI2 -- the device's range, everywhere
    apply(QStringLiteral("output gain %1").arg(output), [o, v](FfiCore *c) { return dspi_set_output_gain(c, o, v); });
}

void ChannelController::setOutputDelay(int output, double ms) {
    if (!writable() || output < 0 || output >= numOutputs()) return;
    // Past the line or not a number: the core refuses it with its own words
    // (the delay-refusal banner), so it is sent as given
    const auto o = uint8_t(output);
    const float v = float(ms);
    apply(QStringLiteral("output delay %1").arg(output), [o, v](FfiCore *c) { return dspi_set_output_delay(c, o, v); });
}

void ChannelController::setOutputMute(int output, bool muted) {
    if (!writable() || output < 0 || output >= numOutputs()) return;
    const auto o = uint8_t(output);
    apply(QStringLiteral("output mute %1").arg(output), [o, muted](FfiCore *c) { return dspi_set_output_mute(c, o, muted); });
}

// PDM (the last output) and the EQ worker outputs (3 up to the one before
// PDM) share Core 1: only one side runs at a time
bool ChannelController::enableWouldConflict(int output) const {
    const DspState *s = m_ctx.device->state();
    const int pdm = pdmOutput();
    if (!s || pdm < 3) return false;
    if (output == pdm) {
        for (int o = 2; o < pdm; ++o)
            if (s->output_enabled[o]) return true;
        return false;
    }
    return output >= 2 && output < pdm && s->output_enabled[pdm];
}

bool ChannelController::requestOutputEnable(int output, bool enabled) {
    if (!writable() || output < 0 || output >= numOutputs()) return true;
    if (enabled && enableWouldConflict(output)) {
        const int pdm = pdmOutput();
        if (output == pdm)
            emit core1Question(output, tr("Warning"), tr("Outputs %1-%2 will be disabled. Are you sure?").arg(3).arg(pdm),
                               tr("Enable PDM"));
        else
            emit core1Question(output, tr("Warning"), tr("The PDM output will be disabled. Are you sure?"),
                               tr("Disable PDM"));
        return false;
    }
    const auto o = uint8_t(output);
    apply(QStringLiteral("output enable %1").arg(output),
          [o, enabled](FfiCore *c) { return dspi_set_output_enable(c, o, enabled) == (enabled ? 1 : 0); });
    return true;
}

void ChannelController::confirmOutputEnable(int output) {
    if (!writable() || output < 0 || output >= numOutputs()) return;
    const int pdm = pdmOutput();
    const auto o = uint8_t(output), p = uint8_t(pdm);
    // The other side goes off first, then this one on, in one worker job
    apply(QStringLiteral("output enable %1").arg(output), [o, p](FfiCore *c) {
        if (o == p) {
            for (uint8_t e = 2; e < p; ++e)
                if (dspi_set_output_enable(c, e, false) < 0) return false;
        } else if (dspi_set_output_enable(c, p, false) < 0) {
            return false;
        }
        return dspi_set_output_enable(c, o, true) == 1;
    });
}

void ChannelController::setLimiterEnabled(int output, bool enabled) {
    if (!writable() || !limiterSupported() || output < 0 || output >= numOutputs()) return;
    const auto o = uint8_t(output);
    // -- WP QPI2 -- a limiter belongs to the output configuration: in
    // Independent mode the save bar offers to keep it (Components.swift)
    if (auto *hw = Registrar::instance<HardwareController>()) hw->beginOutputEdit();
    apply(QStringLiteral("output limiter %1").arg(output),
          [o, enabled](FfiCore *c) { return dspi_set_limiter_param(c, o, LIMITER_PARAM_ENABLED, enabled ? 1.0f : 0.0f); });
}

void ChannelController::requestLimiterSettings(int output) { emit limiterSettingsRequested(output); }

QVariantMap ChannelController::route(int input, int output) const {
    const DspState *s = m_ctx.device->state();
    if (!s || input < 0 || input >= MAX_INPUTS || output < 0 || output >= MAX_OUTPUTS) return {};
    return { { QStringLiteral("enabled"), s->matrix_routing[input][output] },
             { QStringLiteral("gain"), double(s->matrix_gain[input][output]) },
             { QStringLiteral("invert"), s->matrix_invert[input][output] } };
}

void ChannelController::setRoute(int input, int output, bool enabled, double gain, bool invert) {
    if (!writable() || input < 0 || input >= numInputs() || output < 0 || output >= numOutputs() || !std::isfinite(gain))
        return;
    const auto i = uint8_t(input), o = uint8_t(output);
    const float g = float(gain);
    apply(QStringLiteral("matrix route %1 %2").arg(input).arg(output),
          [i, o, enabled, g, invert](FfiCore *c) { return dspi_set_matrix_route(c, i, o, enabled, g, invert); });
}
