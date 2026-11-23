// -- WP QP4 -- The on-graph editor's model and its shared selection;
// PeqEditorController.h says what each part holds.
#include "PeqEditorController.h"

#include "ChannelController.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "SessionState.h"

#include <QCoreApplication>
#include <QRegularExpression>
#include <QVariantMap>
#include <algorithm>
#include <cmath>

static const Registrar::Controller<PeqEditorController> peqEditorController;

namespace Peq {

namespace {
// Shape by shape: the type at order 2 and at order 1 (0 for none)
const struct {
    int second, first;
} kShapeTypes[ShapeCount] = {
    { FILTER_PEAKING, 0 },
    { FILTER_LOWSHELF, FILTER_LOWSHELF1 },
    { FILTER_HIGHPASS, FILTER_HIGHPASS1 },
    { FILTER_HIGHSHELF, FILTER_HIGHSHELF1 },
    { FILTER_LOWPASS, FILTER_LOWPASS1 },
    { FILTER_NOTCH, 0 },
    { FILTER_ALLPASS, FILTER_ALLPASS1 },
};
} // namespace

int typeFor(int shape, int order) {
    if (shape < 0 || shape >= ShapeCount) return -1;
    const int t = order == 2 ? kShapeTypes[shape].second : order == 1 ? kShapeTypes[shape].first : 0;
    return t > 0 ? t : -1;
}

bool shapeOf(int type, int *shape, int *order) {
    for (int s = 0; s < ShapeCount; ++s)
        for (int o : { 2, 1 })
            if (typeFor(s, o) == type) {
                if (shape) *shape = s;
                if (order) *order = o;
                return true;
            }
    return false;
}

QString shapeTitle(int shape) {
    switch (shape) {
    case Bell: return QCoreApplication::translate("PeqEditor", "Bell");
    case LowShelf: return QCoreApplication::translate("PeqEditor", "Low Shelf");
    case LowCut: return QCoreApplication::translate("PeqEditor", "Low Cut");
    case HighShelf: return QCoreApplication::translate("PeqEditor", "High Shelf");
    case HighCut: return QCoreApplication::translate("PeqEditor", "High Cut");
    case Notch: return QCoreApplication::translate("PeqEditor", "Notch");
    case AllPass: return QCoreApplication::translate("PeqEditor", "All Pass");
    default: return {};
    }
}

QString shapeCode(int shape) {
    static const char *const codes[ShapeCount] = { "PK", "LS", "LC", "HS", "HC", "NT", "AP" };
    return shape >= 0 && shape < ShapeCount ? QString::fromLatin1(codes[shape]) : QString();
}

QString orderTitle(int shape, int order) {
    if (shape == AllPass) return order == 1 ? QCoreApplication::translate("PeqEditor", "1st Order") : QCoreApplication::translate("PeqEditor", "2nd Order");
    return order == 1 ? QCoreApplication::translate("PeqEditor", "6 dB/oct") : QCoreApplication::translate("PeqEditor", "12 dB/oct");
}

QString orderLabel(int shape, int order) {
    if (shape == AllPass) return order == 1 ? QCoreApplication::translate("PeqEditor", "180°") : QCoreApplication::translate("PeqEditor", "360°");
    return order == 1 ? QCoreApplication::translate("PeqEditor", "6 dB") : QCoreApplication::translate("PeqEditor", "12 dB");
}

QString orderHelp(int shape, int order) {
    if (shape == AllPass) return order == 1 ? QCoreApplication::translate("PeqEditor", "First order, 180° of phase") : QCoreApplication::translate("PeqEditor", "Second order, 360° of phase");
    return order == 1 ? QCoreApplication::translate("PeqEditor", "6 dB per octave") : QCoreApplication::translate("PeqEditor", "12 dB per octave");
}

bool isCrossover(int type) { return type >= FILTER_XOVER_FIRST && type <= FILTER_XOVER_LAST; }

bool usesGain(int type) {
    return type == FILTER_PEAKING || type == FILTER_LOWSHELF || type == FILTER_HIGHSHELF || type == FILTER_LOWSHELF1
        || type == FILTER_HIGHSHELF1;
}

bool usesQ(int type) {
    switch (type) {
    case FILTER_FLAT:
    case FILTER_LINKWITZ_TRANSFORM:
    case FILTER_ALLPASS1:
    case FILTER_LOWSHELF1:
    case FILTER_HIGHSHELF1:
    case FILTER_LOWPASS1:
    case FILTER_HIGHPASS1: return false;
    default: return !isCrossover(type);
    }
}

bool isGraphBand(const FilterParams &p) { return p.filter_type != FILTER_FLAT && !isCrossover(p.filter_type); }

double response(const FilterParams *bands, int n, double hz) {
    return n > 0 ? double(dspi_compute_response(bands, uint32_t(n), float(hz))) : 0.0;
}

double bandDb(FilterParams p, double hz) {
    p.bypass = false;
    return response(&p, 1, hz);
}

Role roleOf(const FilterParams &p) {
    Role r;
    switch (p.filter_type) {
    case FILTER_PEAKING:
    case FILTER_LOWSHELF:
    case FILTER_HIGHSHELF:
    case FILTER_LOWSHELF1:
    case FILTER_HIGHSHELF1: {
        // Measured, so the dot stays on the curve whatever the shelf's formula
        FilterParams probe = p;
        probe.gain = 12;
        r.kind = Role::Gain;
        r.value = qMax(std::abs(bandDb(probe, p.freq) / 12.0), 0.05);
        break;
    }
    case FILTER_LOWPASS:
    case FILTER_HIGHPASS: r.kind = Role::Resonance; break;
    case FILTER_LOWPASS1:
    case FILTER_HIGHPASS1:
        r.kind = Role::Fixed;
        r.value = bandDb(p, p.freq);
        break;
    case FILTER_LINKWITZ_TRANSFORM:
        r.kind = Role::Locked;
        r.value = bandDb(p, p.freq);
        break;
    default: r.kind = Role::Fixed; break;
    }
    return r;
}

double dotDb(const Role &r, const FilterParams &p) {
    switch (r.kind) {
    case Role::Gain: return double(p.gain) * r.value;
    case Role::Resonance: return 20.0 * std::log10(qMax(double(p.q), 0.01));
    default: return r.value;
    }
}

double clamp(double v, double lo, double hi) { return std::isfinite(v) ? qBound(lo, v, hi) : lo; }

FilterParams flatBand() { return make(FILTER_FLAT, 1000, 0.707, 0); }

FilterParams make(int type, double freq, double q, double gain) {
    FilterParams p{};
    p.filter_type = uint8_t(type);
    p.freq = float(freq);
    p.q = float(q);
    p.gain = float(gain);
    p.qp = 0.707f;
    return p;
}

bool same(const FilterParams &a, const FilterParams &b) {
    return a.filter_type == b.filter_type && a.bypass == b.bypass && a.freq == b.freq && a.q == b.q && a.gain == b.gain
        && a.qp == b.qp;
}

FilterParams blend(const FilterParams &from, const FilterParams &to, double t) {
    if (from.filter_type != to.filter_type || from.bypass != to.bypass || t >= 1) return to;
    FilterParams p = to;
    auto logMix = [t](double a, double b) {
        a = qMax(a, 0.01);
        b = qMax(b, 0.01);
        return std::exp(std::log(a) + (std::log(b) - std::log(a)) * t);
    };
    p.freq = float(logMix(from.freq, to.freq));
    p.q = float(logMix(from.q, to.q));
    p.gain = float(from.gain + (to.gain - from.gain) * t);
    return p;
}

QString frequencyText(double hz) {
    // -- WP QPI2 -- the units through the translation file
    if (hz >= 10000) return QCoreApplication::translate("Peq", "%1 kHz").arg(hz / 1000, 0, 'f', 2);
    if (hz >= 1000) return QCoreApplication::translate("Peq", "%1 kHz").arg(hz / 1000, 0, 'f', 3);
    return QCoreApplication::translate("Peq", "%1 Hz").arg(hz, 0, 'f', hz >= 100 ? 1 : 2);
}

QString shortFrequency(double hz) {
    if (hz >= 1000) return QString::number(hz / 1000, 'f', hz >= 10000 ? 1 : 2) + QLatin1Char('k');
    return QString::number(hz, 'f', hz >= 100 ? 0 : 1);
}

QString gainText(double db) {
    const double v = std::abs(db) < 0.005 ? 0.0 : db;
    return (v >= 0 ? QStringLiteral("+") : QString()) + QString::number(v, 'f', 2) + QStringLiteral(" dB");
}

QString truncated(double v, bool sign) {
    const double t = std::floor(std::abs(v) * 100 + 1e-5) / 100;
    const QString text = QString::number(t, 'f', 2);
    if (t == 0) return sign ? QLatin1Char('+') + text : text;
    return v < 0 ? QLatin1Char('-') + text : (sign ? QLatin1Char('+') + text : text);
}

bool parseNote(const QString &text, double *hz) {
    // C4 = middle C; "A4" = 440 Hz; "C#2+13" is C sharp 2 raised 13 cents
    static const QRegularExpression note(QStringLiteral("^([a-g])(#|b(?=-?\\d))?(-?\\d+)([+-]\\d+(?:\\.\\d+)?)?$"));
    const QRegularExpressionMatch m = note.match(text);
    if (!m.hasMatch()) return false;
    static const int base[] = { 9, 11, 0, 2, 4, 5, 7 };   // a..g
    int semitone = base[m.captured(1).at(0).unicode() - 'a'];
    if (m.captured(2) == QLatin1String("#")) semitone += 1;
    else if (m.captured(2) == QLatin1String("b")) semitone -= 1;
    const double cents = m.captured(4).isEmpty() ? 0.0 : m.captured(4).toDouble();
    const double midi = (m.captured(3).toInt() + 1) * 12 + semitone + cents / 100.0;
    *hz = 440.0 * std::pow(2.0, (midi - 69.0) / 12.0);
    return true;
}

// -- WP USR1 -- A typed number as BorderedField.number reads it: no spaces
// (no-break and thin ones too), the minus sign U+2212 as "-", one comma as the
// decimal point. A comma beside a point stays, so toDouble refuses it.
static QString plainNumber(QString s) {
    s.remove(QRegularExpression(QStringLiteral("[\\s\\x{00a0}\\x{202f}\\x{2009}]")));
    s.replace(QRegularExpression(QStringLiteral("[\\x{2212}\\x{2012}\\x{2013}\\x{fe63}\\x{ff0d}]")), QStringLiteral("-"));
    if (s.count(QLatin1Char(',')) == 1 && !s.contains(QLatin1Char('.'))) s.replace(QLatin1Char(','), QLatin1Char('.'));
    return s;
}

bool parseFrequency(const QString &text, double *hz) {
    QString s = text.trimmed().toLower();
    if (parseNote(s, hz)) return true;
    s = plainNumber(s);   // -- WP USR1 --
    double scale = 1;
    if (s.endsWith(QLatin1String("khz"))) {
        scale = 1000;
        s.chop(3);
    } else if (s.endsWith(QLatin1String("hz"))) {
        s.chop(2);
    }
    if (s.endsWith(QLatin1Char('k'))) {
        scale = 1000;
        s.chop(1);
    }
    bool ok = false;
    const double v = s.toDouble(&ok);
    if (!ok || !std::isfinite(v)) return false;
    *hz = v * scale;
    return true;
}

bool parseNumber(const QString &text, const QString &unit, double *v) {
    QString s = plainNumber(text.trimmed().toLower());   // -- WP USR1 --
    if (!unit.isEmpty() && s.endsWith(unit)) s.chop(unit.size());
    if (s.startsWith(QLatin1Char('q'))) s.remove(0, 1);
    if (s.startsWith(QLatin1Char('+'))) s.remove(0, 1);
    bool ok = false;
    const double x = s.toDouble(&ok);
    if (!ok || !std::isfinite(x)) return false;
    *v = x;
    return true;
}

} // namespace Peq

namespace {
ChannelController *channels() { return Registrar::instance<ChannelController>(); }
} // namespace

PeqEditorController::PeqEditorController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    // -- WP MDQT2a --
    connect(ctx.registry, &DeviceRegistry::activeLeaving, this, &PeqEditorController::letGo);
    connect(ctx.registry, &DeviceRegistry::activeChanged, this, [this] {
        m_switching = false;
        emit selectionChanged();
        bump();
    });
}

// -- WP MDQT2a --
std::shared_ptr<PeqEditorController::State> PeqEditorController::state() const {
    return sessionState<State>(m_ctx.activeSession(), QStringLiteral("PeqEditorController"));
}

void PeqEditorController::letGo() {
    m_switching = true;
    m_pointed = -1;
    setRowHovered(-1);
    endLive();
}

void PeqEditorController::bump() {
    ++m_rev;
    emit changed();
}

QList<int> PeqEditorController::selected() const {
    const QSet<int> &s = selection();
    QList<int> out(s.cbegin(), s.cend());
    std::sort(out.begin(), out.end());
    return out;
}

void PeqEditorController::setRowHovered(int band) {
    if (band == m_rowHovered) return;
    m_rowHovered = band;
    emit rowHoveredChanged();
    bump();
}

void PeqEditorController::setPointedBand(int band) {
    if (band == m_pointed) return;
    m_pointed = band;
    bump();
}

void PeqEditorController::setEditedChannel(int channel) {
    const std::shared_ptr<State> st = state();
    if (channel == st->edited) return;
    st->edited = channel;
    // -- WP MDQT2a -- no channel while the pages move to another device's is
    // not the user leaving this one: its selection waits for it
    const bool keeps = channel < 0 ? m_switching : channel == st->selectedOn;
    if (!keeps) {
        st->selected.clear();
        st->rowAnchor = -1;
        st->selectedOn = channel;
    }
    m_rowHovered = m_pointed = -1;
    m_live.clear();
    emit selectionChanged();
    bump();
}

void PeqEditorController::setSelection(const QSet<int> &bands) {
    const std::shared_ptr<State> st = state();
    if (bands == st->selected) return;
    st->selected = bands;
    bump();
}

void PeqEditorController::forget() {
    const std::shared_ptr<State> st = state();
    st->rowAnchor = -1;
    const bool any = !st->selected.isEmpty() || m_pointed >= 0 || m_rowHovered >= 0;
    st->selected.clear();
    m_pointed = m_rowHovered = -1;
    if (!any) return;
    emit selectionChanged();
    emit rowHoveredChanged();
    bump();
}

// -- WP LAY4 --
int PeqEditorController::shapeOfType(int type) const {
    int shape = -1;
    return Peq::shapeOf(type, &shape, nullptr) ? shape : -1;
}

void PeqEditorController::rowClicked(int band, int modifiers) {
    const std::shared_ptr<State> st = state();
    if (st->edited < 0 || band < 0 || band >= BANDS_PER_CHANNEL) return;
    QSet<int> next;
    int from = st->rowAnchor >= 0 && st->selected.contains(st->rowAnchor) ? st->rowAnchor
             : st->selected.size() == 1                                    ? *st->selected.cbegin()
                                                                           : -1;
    if (modifiers & Qt::ControlModifier) {
        next = st->selected;
        if (!next.remove(band)) next.insert(band);
        st->rowAnchor = band;
    } else if ((modifiers & Qt::ShiftModifier) && from >= 0) {
        // The anchor stays, so a second Shift-click reshapes the run
        for (int b = qMin(from, band); b <= qMax(from, band); ++b)
            if (Peq::isGraphBand(this->band(st->edited, b, false))) next.insert(b);
        st->rowAnchor = from;
    } else {
        next = { band };
        st->rowAnchor = band;
    }
    if (next == st->selected) return;
    st->selected = next;
    emit selectionChanged();
    bump();
}

QVariant PeqEditorController::live(int band) const {
    const auto it = m_live.constFind(band);
    if (it == m_live.cend()) return {};
    return QVariantMap{ { QStringLiteral("freq"), double(it->freq) },
                        { QStringLiteral("gain"), double(it->gain) },
                        { QStringLiteral("q"), double(it->q) } };
}

void PeqEditorController::showLive(int band, const FilterParams &p) {
    m_live.insert(band, p);
    bump();
}

void PeqEditorController::endLive() {
    if (m_live.isEmpty()) return;
    m_live.clear();
    bump();
}

FilterParams PeqEditorController::band(int channel, int band, bool crossover) const {
    const QVariantMap m = channels()->band(channel, band, crossover);
    FilterParams p = Peq::make(m.value(QStringLiteral("type")).toInt(), m.value(QStringLiteral("freq")).toDouble(),
                               m.value(QStringLiteral("q")).toDouble(), m.value(QStringLiteral("gain")).toDouble());
    p.qp = float(m.value(QStringLiteral("qp")).toDouble());
    p.bypass = m.value(QStringLiteral("bypass")).toBool();
    return p;
}

bool PeqEditorController::writable() const { return channels()->writable(); }
bool PeqEditorController::bypassSupported() const { return channels()->bypassSupported(); }

QList<int> PeqEditorController::availableTypes() const {
    QList<int> out;
    for (const QVariant &t : channels()->availableTypes(false)) out << t.toInt();
    return out;
}

void PeqEditorController::writeBands(int channel, const QList<QPair<int, FilterParams>> &changes) {
    // ChannelController sets one field per call; each call stages the band,
    // so the next one builds on it, and they share the band's coalescing key
    ChannelController *cc = channels();
    for (const auto &[b, p] : changes) {
        const FilterParams now = band(channel, b, false);
        if (now.filter_type != p.filter_type) cc->setBandType(channel, b, false, p.filter_type);
        if (now.freq != p.freq) cc->setBandValue(channel, b, false, QStringLiteral("freq"), p.freq);
        if (now.q != p.q) cc->setBandValue(channel, b, false, QStringLiteral("q"), p.q);
        if (band(channel, b, false).gain != p.gain) cc->setBandValue(channel, b, false, QStringLiteral("gain"), p.gain);
        if (now.bypass != p.bypass && cc->bypassSupported()) cc->setBandBypass(channel, b, false, p.bypass);
    }
}

void PeqEditorController::setBandBypassed(int channel, int band, bool bypass) {
    if (channels()->bypassSupported()) channels()->setBandBypass(channel, band, false, bypass);
}

// ---- -- WP LAY3b -- The band menu ------------------------------------------------

FilterParams Peq::reshaped(FilterParams p, int type) {
    const FilterParams was = p;
    int shape = -1;
    Peq::shapeOf(type, &shape, nullptr);
    p.filter_type = uint8_t(type);
    // Leaving the Linkwitz Transform drops its fp back to 0 dB
    if (was.filter_type == FILTER_LINKWITZ_TRANSFORM || !Peq::usesGain(type)) p.gain = 0;
    if (Peq::usesQ(type) && !Peq::usesQ(was.filter_type)) p.q = shape == Peq::Bell || shape == Peq::Notch ? 1.0f : 0.707f;
    return p;
}

QVariantList PeqEditorController::shapes() const {
    const QList<int> available = availableTypes();
    QVariantList out;
    for (int s = 0; s < Peq::ShapeCount; ++s)
        for (int o : { 2, 1 }) {
            if (!available.contains(Peq::typeFor(s, o))) continue;
            out << QVariantMap{ { QStringLiteral("shape"), s }, { QStringLiteral("title"), Peq::shapeTitle(s) } };
            break;
        }
    return out;
}

QVariantMap PeqEditorController::shapeOrder(int type) const {
    int shape = -1, order = 2;
    if (!Peq::shapeOf(type, &shape, &order)) shape = -1, order = 2;
    return { { QStringLiteral("shape"), shape }, { QStringLiteral("order"), order } };
}

QString PeqEditorController::slopeTitle(int type) const {
    int shape = -1;
    Peq::shapeOf(type, &shape, nullptr);
    return shape == Peq::AllPass ? tr("Order") : tr("Slope");
}

QVariantList PeqEditorController::slopes(int type) const {
    int shape = -1, order = 0;
    if (!Peq::shapeOf(type, &shape, &order)) return {};
    const QList<int> available = availableTypes();
    if (!available.contains(Peq::typeFor(shape, 1)) || !available.contains(Peq::typeFor(shape, 2))) return {};
    QVariantList out;
    for (int o : { 1, 2 })
        out << QVariantMap{ { QStringLiteral("name"), o },
                            { QStringLiteral("text"), Peq::orderTitle(shape, o) },
                            { QStringLiteral("value"), QVariantMap{ { QStringLiteral("shape"), shape }, { QStringLiteral("order"), o } } },
                            { QStringLiteral("checked"), o == order } };
    return out;
}

void PeqEditorController::setShape(int channel, const QList<int> &bands, int shape, int order) {
    const QList<int> available = availableTypes();
    int type = Peq::typeFor(shape, order);
    if (type < 0 || !available.contains(type)) type = Peq::typeFor(shape, order == 2 ? 1 : 2);
    if (type < 0 || !available.contains(type) || !writable()) return;
    QList<QPair<int, FilterParams>> changes;
    for (int b : bands) changes.append({ b, Peq::reshaped(band(channel, b, false), type) });
    writeBands(channel, changes);
}

void PeqEditorController::bypassBands(int channel, const QList<int> &bands, bool crossover, bool bypass) {
    if (!bypassSupported() || !writable()) return;
    for (int b : bands)
        if (band(channel, b, crossover).filter_type != FILTER_FLAT) channels()->setBandBypass(channel, b, crossover, bypass);
}

void PeqEditorController::invertGain(int channel, const QList<int> &bands) {
    if (!writable()) return;
    QList<QPair<int, FilterParams>> changes;
    for (int b : bands) {
        FilterParams p = band(channel, b, false);
        if (!Peq::usesGain(p.filter_type)) continue;
        p.gain = -p.gain;
        changes.append({ b, p });
    }
    writeBands(channel, changes);
}

void PeqEditorController::resetBands(int channel, const QList<int> &bands, bool crossover) {
    if (!writable()) return;
    if (crossover) {
        for (int b : bands) channels()->setCrossoverFamily(channel, b, 0);
        return;
    }
    QList<QPair<int, FilterParams>> changes;
    const std::shared_ptr<State> st = state();
    QSet<int> rest = st->selected;
    for (int b : bands) {
        changes.append({ b, Peq::flatBand() });
        rest.remove(b);
    }
    // An Off band has no dot to be selected
    if (st->edited == channel && rest != st->selected) {
        st->selected = rest;
        emit selectionChanged();
        bump();
    }
    writeBands(channel, changes);
}

void PeqEditorController::copyBand(int channel, int b) {
    const FilterParams p = band(channel, b, false);
    if (p.filter_type == FILTER_FLAT) return;
    m_copied = p;
    m_hasBand = true;
    emit bandCopied();
}

void PeqEditorController::pasteBand(int channel, const QList<int> &bands) {
    if (!m_hasBand || !writable()) return;
    ChannelController *cc = channels();
    const bool output = channel >= cc->numInputs();
    // Another device may not have the type; the Linkwitz Transform is an output's
    if (!cc->availableTypes(output).contains(int(m_copied.filter_type))) return;
    if (m_copied.filter_type == FILTER_LINKWITZ_TRANSFORM) {
        for (int b : bands) {
            cc->setBandType(channel, b, false, FILTER_LINKWITZ_TRANSFORM);
            cc->setLinkwitz(channel, b, m_copied.freq, m_copied.q, m_copied.gain, m_copied.qp);
            if (cc->bypassSupported()) cc->setBandBypass(channel, b, false, m_copied.bypass);
        }
        return;
    }
    QList<QPair<int, FilterParams>> changes;
    for (int b : bands) changes.append({ b, m_copied });
    writeBands(channel, changes);
}
