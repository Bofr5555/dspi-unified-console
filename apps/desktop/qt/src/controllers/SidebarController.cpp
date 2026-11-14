// -- WP QP1 -- The sidebar's state and writes (SidebarController.h).
#include "SidebarController.h"
#include "Confirm.h"   // -- WP P1QT2 --

#include "ChannelController.h"
#include "Core.h"
#include "DeviceModel.h"
#include "Shell.h"
#include "SiggenController.h"
#include "Snapshot.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QSettings>
#include <QTimer>
#include <QWindow>
#include <cmath>
#include <memory>

static const Registrar::Controller<SidebarController> sidebarController;

namespace {

// The firmware's input source values (the Mac's INPUT_SOURCE_*)
enum : int { SourceUsb = 0, SourceSpdif = 1, SourceI2s = 2, SourceAdat = 3, SourceSpdif2 = 4, SourceLast = 6 };
constexpr int kSourceNameBytes = 32;
constexpr uint8_t kAdatPinUnset = 0xFF;
constexpr uint8_t kPresenceSpdifInputs = 0x02;
constexpr uint8_t kPresetLocked = 0xFE;
constexpr int kSummaryLines = 15;

// The windows the quick strip opens and shows as active
const QStringList kStripWindows = { QStringLiteral("tools.matrix"), QStringLiteral("tools.stats"),
                                    QStringLiteral("edit.settings") };

// The master taper's regions: 0.1 dB steps down to -10 dB over 100 units,
// 0.5 dB down to -40 over 60, 1 dB down to mute over 88
constexpr double kMasterUnits = 248.0;
constexpr double kMasterBreak1 = 1.0 - 100.0 / kMasterUnits;
constexpr double kMasterBreak2 = kMasterBreak1 - 60.0 / kMasterUnits;
} // namespace

SidebarController::SidebarController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    m_masterMode = QSettings().value(QStringLiteral("sidebar/showMaster"), false).toBool();
    m_sidebarWidth = qMax(0, QSettings().value(QStringLiteral("sidebar/width"), 0).toInt());
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &SidebarController::onSnapshot);
    connect(ctx.shell, &Shell::triggered, this, [this](const QString &action) {
        if (kStripWindows.contains(action)) QTimer::singleShot(0, this, [this, action]() { trackWindow(action); });
    });
    // Edit > Copy / Paste with no text field focused: the selected channel's
    // parameters (MenuTable.trigger gives a focused field the text first)
    ctx.shell->addHandler(QStringLiteral("edit.copy"), this, [this]() {
        if (m_selected >= 0) copyChannel(m_selected);
    });
    ctx.shell->addHandler(QStringLiteral("edit.paste"), this, [this]() {
        if (m_selected >= 0) pasteChannel(m_selected);
    });
}

// ---- Tapers --------------------------------------------------------------------

// The user volume: a square-root taper over -60..0 dB, more travel near the top
double SidebarController::userPositionToDb(double position) {
    if (!(position > 0)) return kUserMinDb;
    return kUserMinDb + -kUserMinDb * std::sqrt(qMin(1.0, position));
}

double SidebarController::userDbToPosition(double db) {
    if (!(db > kUserMinDb)) return 0;
    const double fraction = (db - kUserMinDb) / -kUserMinDb;
    return qMin(1.0, fraction * fraction);
}

double SidebarController::masterPositionToDb(double position) {
    if (!(position > 0)) return kMasterMuteDb;
    if (position >= 1) return 0;
    if (position > kMasterBreak1) return -(1.0 - position) * kMasterUnits * 0.1;
    if (position > kMasterBreak2) return -10.0 - (kMasterBreak1 - position) * kMasterUnits * 0.5;
    return -40.0 - (kMasterBreak2 - position) * kMasterUnits;
}

double SidebarController::masterDbToPosition(double db) {
    if (!(db > kMasterMuteDb)) return 0;
    if (db >= 0) return 1;
    if (db > -10) return 1.0 - (-db / 0.1) / kMasterUnits;
    if (db > -40) return kMasterBreak1 - ((-db - 10) / 0.5) / kMasterUnits;
    return kMasterBreak2 - (-db - 40) / kMasterUnits;
}

QString SidebarController::formatMasterDb(double db) {
    if (!(db > kMasterMuteDb)) return QStringLiteral("−∞");
    double shown = db > -10 ? std::round(db * 10) / 10 : db > -40 ? std::round(db * 2) / 2 : std::round(db);
    if (shown == 0) shown = 0; // no "-0"
    QString text = QString::number(shown, 'f', 1);
    if (text.endsWith(QLatin1String(".0"))) text.chop(2);
    return text + QStringLiteral(" dB");
}

QString SidebarController::formatUserDb(double db) { return QString::number(db, 'f', 1) + QStringLiteral(" dB"); }

QString SidebarController::sanitizeSourceName(const QString &name) {
    QString clean;
    for (const QChar c : name) {
        if (c.unicode() >= 32 && c.unicode() < 127) clean.append(c);
        if (clean.size() == kSourceNameBytes - 1) break;
    }
    return clean;
}

// ---- Reading the device --------------------------------------------------------

double SidebarController::userVolumeDb() const {
    const DspState *s = m_ctx.device->state();
    return s ? double(s->user_volume_db) : 0.0;
}

double SidebarController::masterVolumeDb() const {
    const DspState *s = m_ctx.device->state();
    return s ? double(s->master_volume_db) : 0.0;
}

bool SidebarController::crossfeedOn() const { return m_ctx.device->state() && m_ctx.device->state()->crossfeed_enabled; }
bool SidebarController::loudnessOn() const { return m_ctx.device->state() && m_ctx.device->state()->loudness_enabled; }
bool SidebarController::levellerOn() const { return m_ctx.device->state() && m_ctx.device->state()->leveller_enabled; }
bool SidebarController::psybassOn() const { return m_ctx.device->state() && m_ctx.device->state()->psybass_enabled; }
bool SidebarController::bypassOn() const { return m_ctx.device->state() && m_ctx.device->state()->bypass; }

// Hidden, not disabled, on firmware without it (behaviour.md 14.1)
bool SidebarController::psybassSupported() const {
    return !m_ctx.device->connected() || (m_ctx.device->snapshot().wireFeatures & WIRE_FEAT_PSYBASS) != 0;
}

int SidebarController::inactiveOutputs() const {
    const DspState *s = m_ctx.device->state();
    int mask = 0;
    for (int o = 0; s && o < MAX_OUTPUTS; ++o)
        if (!s->output_enabled[o] || s->output_muted[o]) mask |= 1 << o;
    return mask;
}

QStringList SidebarController::slotNames() const {
    QStringList names;
    for (const QJsonValue &v : m_ctx.device->snapshot().outputLabels) {
        const QJsonObject o = v.toObject();
        const int output = o.value(QLatin1String("output")).toInt();
        while (names.size() <= output && output < MAX_OUTPUTS) names.append(QString());
        if (output >= 0 && output < names.size()) names[output] = o.value(QLatin1String("slotName")).toString();
    }
    return names;
}

// The Mac widens the name column (80 to 112) while an output shows the
// core's "Out 2 · R · High" rather than a stored name
bool SidebarController::derivedLabels() const {
    const ChannelListModel *rows = m_ctx.device->channels();
    for (int r = 0; r < rows->rowCount(); ++r) {
        if (rows->get(r, QStringLiteral("kind")).toString() != QLatin1String("output")) continue;
        if (rows->get(r, QStringLiteral("outputEnabled")).toBool() &&
            rows->get(r, QStringLiteral("label")) != rows->get(r, QStringLiteral("name")))
            return true;
    }
    return false;
}

bool SidebarController::sourceSupported() const { return m_ctx.device->state() != nullptr; }
int SidebarController::source() const { return m_ctx.device->state() ? m_ctx.device->state()->input_source : SourceUsb; }

QString SidebarController::selectedKind() const {
    if (m_selected < 0) return QStringLiteral("overview");
    return m_selected < m_ctx.device->channels()->inputCount() ? QStringLiteral("input") : QStringLiteral("output");
}

int SidebarController::selectedIndex() const {
    if (m_selected < 0) return -1;
    const ChannelListModel *rows = m_ctx.device->channels();
    const int row = rows->rowOfChannel(m_selected);
    return row < 0 ? -1 : rows->get(row, QStringLiteral("number")).toInt();
}

void SidebarController::onSnapshot() {
    DeviceModel *device = m_ctx.device;
    watchLinks();
    // A page whose row went away (no device, the output switched off) goes back
    // to the overview, as the Mac's sidebar does
    if (m_selected >= 0) {
        const ChannelListModel *rows = device->channels();
        const int row = rows->rowOfChannel(m_selected);
        if (!device->connected() || row < 0 || !rows->get(row, QStringLiteral("outputEnabled")).toBool()) select(-1);
    }
    updateHighlight();
    emit deviceChanged();
    rebuildSources();
    readExtras();
}

// The preset flag and lines, the source names and the presence bits are kept
// by the core and read without device traffic; the generator's capabilities
// once per connection.
void SidebarController::readExtras() {
    if (m_extrasInFlight) {
        m_extrasAgain = true;
        return;
    }
    if (!m_ctx.device->connected()) {
        applyExtras(Extras{});
        return;
    }
    m_extrasInFlight = true;
    const uint64_t generation = m_ctx.device->snapshot().generation;
    const bool needCaps = generation != m_capsGeneration;
    m_ctx.core->worker()->call(
        this,
        [needCaps](FfiCore *c) {
            Extras e;
            e.dirty = dspi_preset_dirty(c);
            const QJsonArray lines =
                QJsonDocument::fromJson(readSized([c](char *b, uint32_t n) { return dspi_preset_changes(c, b, n); }))
                    .array();
            for (const QJsonValue &v : lines) e.changes.append(v.toString());
            char name[kSourceNameBytes + 1] = {};
            e.namesSupported = dspi_cached_input_source_name(c, 0, name, sizeof name);
            for (int s = 0; s <= SourceLast; ++s) {
                name[0] = 0;
                e.sourceNames.append(e.namesSupported && dspi_cached_input_source_name(c, uint8_t(s), name, sizeof name)
                                         ? QString::fromUtf8(name)
                                         : QString());
            }
            e.presence = dspi_state_presence(c);
            if (needCaps) {
                SiggenCaps caps{};
                e.capsRead = true;
                e.siggenMask = dspi_siggen_fetch_caps(c, &caps) && caps.supported ? caps.valid_mask : 0;
            }
            return e;
        },
        [this, generation](Extras e) {
            m_extrasInFlight = false;
            if (e.capsRead) {
                m_capsGeneration = generation;
                if (e.siggenMask != m_siggenMask) {
                    m_siggenMask = e.siggenMask;
                    emit capsChanged();
                }
            } else {
                e.siggenMask = m_siggenMask;
            }
            applyExtras(e);
            if (m_extrasAgain) {
                m_extrasAgain = false;
                readExtras();
            }
        });
}

void SidebarController::applyExtras(const Extras &e) {
    if (e.dirty != m_unsaved || e.changes != m_unsavedLines) {
        m_unsaved = e.dirty;
        m_unsavedLines = e.changes;
        emit unsavedChanged();
        emit deviceChanged(); // the "*" on the preset's title
    }
    if (!m_ctx.device->connected() && m_siggenMask) {
        m_siggenMask = 0;
        m_capsGeneration = 0;
        emit capsChanged();
    }
    if (e.namesSupported != m_sourceNamesSupported || e.sourceNames != m_sourceNames || e.presence != m_presence) {
        m_sourceNamesSupported = e.namesSupported;
        m_sourceNames = e.sourceNames;
        m_presence = e.presence;
        rebuildSources();
    }
}

// USB, S/PDIF 1, the optional S/PDIF inputs that are switched on, I2S, then
// ADAT once it is on with a pin (DSPViewModel.swift's inputSourceOptions)
void SidebarController::rebuildSources() {
    QVariantList list;
    if (const DspState *s = m_ctx.device->state()) {
        QList<int> values = { SourceUsb, SourceSpdif };
        if (m_presence & kPresenceSpdifInputs)
            for (int i = 1; i < 4; ++i)
                if (s->spdif_inputs_enabled >> i & 1) values.append(SourceSpdif2 + i - 1);
        values.append(SourceI2s);
        if (m_ctx.device->platformName() == QLatin1String("RP2350") && s->adat_input_enabled &&
            s->adat_input_pin != kAdatPinUnset && s->adat_input_pin != 0)
            values.append(SourceAdat);
        for (int v : values) list.append(QVariantMap{ { QStringLiteral("value"), v }, { QStringLiteral("title"), sourceTitle(v) } });
    }
    if (list != m_sources) {
        m_sources = list;
        emit sourcesChanged();
    }
}

// ---- Selection and links --------------------------------------------------------

void SidebarController::select(int channel) {
    if (channel >= 0 && m_ctx.device->channels()->rowOfChannel(channel) < 0) channel = -1;
    if (channel == m_selected) return;
    m_selected = channel;
    emit selectionChanged();
    updateHighlight();
}

void SidebarController::rowClicked(int channel) { select((m_highlighted >> channel & 1) ? -1 : channel); }

// Inputs are wire channels 0..n-1, so an input's number is its channel
int SidebarController::linkedPartner(int channel) const {
    if (channel < 0 || channel >= m_ctx.device->channels()->inputCount()) return -1;
    return Registrar::instance<ChannelController>()->linkedPartner(channel);
}

// A link made or broken on the input page moves the highlight. Looked up on
// first use: controllers are made in no set order.
void SidebarController::watchLinks() {
    if (m_watchingLinks) return;
    m_watchingLinks = true;
    connect(Registrar::instance<ChannelController>(), &ChannelController::changed, this, &SidebarController::updateHighlight);
}

void SidebarController::updateHighlight() {
    int mask = 0;
    if (m_selected >= 0) {
        mask = 1 << m_selected;
        const int partner = linkedPartner(m_selected);
        if (partner >= 0) mask |= 1 << partner;
    }
    if (mask == m_highlighted) return;
    m_highlighted = mask;
    emit highlightChanged();
}

// ---- Writes --------------------------------------------------------------------

void SidebarController::apply(const QString &what, std::function<bool(FfiCore *)> setter, std::function<void(bool)> done) {
    m_ctx.device->write(what, std::move(setter), std::move(done));
}

bool SidebarController::renameChannel(int channel, const QString &name) {
    QString trimmed = name.trimmed();
    if (trimmed.isEmpty()) return false;
    // At most what the device stores, cut at a character
    while (trimmed.toUtf8().size() >= CHANNEL_NAME_LEN) trimmed.chop(1);
    return m_ctx.device->setChannelName(channel, trimmed);
}

void SidebarController::copyChannel(int channel) {
    const DspState *s = m_ctx.device->state();
    if (!s || channel < 0 || channel >= MAX_CHANNELS) return;
    Clipboard cb;
    cb.valid = true;
    const int row = m_ctx.device->channels()->rowOfChannel(channel);
    cb.name = row >= 0 ? m_ctx.device->channels()->get(row, QStringLiteral("label")).toString() : QString();
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b) cb.filters.append(s->filters[channel][b]);
    const int output = channel - int(s->num_input_channels);
    if (output >= 0 && output < MAX_OUTPUTS) {
        cb.fromOutput = true;
        for (int b = 0; b < MAX_XOVER_BANDS; ++b) cb.crossover.append(s->xover[channel][b]);
        cb.gainDb = s->output_gain_db[output];
        cb.delayMs = s->output_delay_ms[output];
        cb.muted = s->output_muted[output];
    }
    m_clipboard = cb;
    emit clipboardChanged();
}

// The bands, mirrored onto a linked partner (the pair must not drift apart);
// crossover, gain, delay and mute only from one output to another
void SidebarController::pasteChannel(int channel) {
    const DspState *s = m_ctx.device->state();
    if (!m_clipboard.valid || !s || channel < 0 || channel >= MAX_CHANNELS) return;
    const Clipboard cb = m_clipboard;
    const int partner = linkedPartner(channel);
    const int output = channel - int(s->num_input_channels);
    const bool toOutput = output >= 0 && output < MAX_OUTPUTS;
    apply(QStringLiteral("paste channel %1").arg(channel), [cb, channel, partner, output, toOutput](FfiCore *c) {
        bool ok = true;
        for (int b = 0; b < BANDS_PER_CHANNEL; ++b) {
            const FilterParams f = b < cb.filters.size() ? cb.filters.at(b) : FilterParams{};
            ok = dspi_set_filter(c, uint8_t(channel), uint8_t(b), f) && ok;
            if (partner >= 0) ok = dspi_set_filter(c, uint8_t(partner), uint8_t(b), f) && ok;
        }
        if (toOutput && cb.fromOutput) {
            for (int b = 0; b < cb.crossover.size(); ++b)
                ok = dspi_set_crossover(c, uint8_t(channel), uint8_t(b), cb.crossover.at(b)) && ok;
            ok = dspi_set_output_gain(c, uint8_t(output), cb.gainDb) && ok;
            // -- WP QP16 -- a delay past this device's line is held at the line (behaviour.md §15)
            ok = dspi_set_output_delay(c, uint8_t(output), qMin(cb.delayMs, dspi_max_delay_ms(c))) && ok;
            ok = dspi_set_output_mute(c, uint8_t(output), cb.muted) && ok;
        }
        return ok;
    });
}

// -- WP QPI3 -- the generator's controller plays it (one owner of its writes)
void SidebarController::identifyOutput(int output) {
    if (output < 0 || output >= 16 || !(uint16_t(1u << output) & m_siggenMask)) return;
    if (auto *siggen = Registrar::instance<SiggenController>()) siggen->playIdent(output);
}

void SidebarController::setEffect(const QString &effect, bool on) {
    std::function<bool(FfiCore *)> setter;
    if (effect == QLatin1String("crossfeed")) setter = [on](FfiCore *c) { return dspi_set_crossfeed(c, on); };
    else if (effect == QLatin1String("loudness")) setter = [on](FfiCore *c) { return dspi_set_loudness(c, on); };
    else if (effect == QLatin1String("leveller")) setter = [on](FfiCore *c) { return dspi_set_leveller_enabled(c, on); };
    else if (effect == QLatin1String("psybass")) setter = [on](FfiCore *c) { return dspi_set_psybass_enabled(c, on); };
    else if (effect == QLatin1String("bypass")) setter = [on](FfiCore *c) { return dspi_set_bypass(c, on); };
    else return;
    apply(effect + QStringLiteral(" enabled"), std::move(setter));
}

void SidebarController::toggleWindow(const QString &action) {
    QWindow *w = m_windows.value(action);
    if (w && w->isVisible()) {
        w->close();
        return;
    }
    m_ctx.shell->trigger(action);
}

// After the shell ran an action of the strip's: the window it opened, if it
// is a window, followed for the button's active look
void SidebarController::trackWindow(const QString &action) {
    if (m_windows.value(action) || !m_ctx.shell->handles(action)) return;
    QWindow *w = m_ctx.shell->openWindow(action);
    if (!w) return;
    m_windows.insert(action, w);
    connect(w, &QWindow::visibleChanged, this, &SidebarController::openWindowsChanged);
    connect(w, &QObject::destroyed, this, &SidebarController::openWindowsChanged);
    emit openWindowsChanged();
}

QStringList SidebarController::openWindows() const {
    QStringList open;
    for (auto it = m_windows.cbegin(); it != m_windows.cend(); ++it)
        if (it.value() && it.value()->isVisible()) open.append(it.key());
    return open;
}

void SidebarController::setVolumeMode(const QString &mode) {
    const bool master = mode == QLatin1String("master");
    if (master == m_masterMode) return;
    m_masterMode = master;
    QSettings().setValue(QStringLiteral("sidebar/showMaster"), master);
    emit volumeModeChanged();
}

void SidebarController::setSidebarWidth(int width) {
    if (width <= 0 || width == m_sidebarWidth) return;
    m_sidebarWidth = width;
    QSettings().setValue(QStringLiteral("sidebar/width"), width);
    emit sidebarWidthChanged();
}

void SidebarController::setUserVolume(double db) {
    const float v = float(qBound(kUserMinDb, db, 0.0));
    apply(QStringLiteral("user volume"), [v](FfiCore *c) { return dspi_set_user_volume(c, v); });
}

void SidebarController::setMasterVolume(double db) {
    const float v = float(qBound(kMasterMuteDb, db, 0.0));
    apply(QStringLiteral("master volume"), [v](FfiCore *c) { return dspi_set_master_volume(c, v); });
}

// ---- Source --------------------------------------------------------------------

void SidebarController::setSource(int value) {
    if (value < 0 || value > SourceLast || value == source()) return;
    apply(QStringLiteral("input source"), [value](FfiCore *c) { return dspi_set_input_source(c, uint8_t(value)); });
}

QString SidebarController::sourceName(int value) const { return value >= 0 && value < m_sourceNames.size() ? m_sourceNames.at(value) : QString(); }

QString SidebarController::sourceTitle(int value) const {
    const QString name = sourceName(value);
    return name.isEmpty() ? builtInSourceTitle(value) : name;
}

QString SidebarController::builtInSourceTitle(int value) const {
    const DspState *s = m_ctx.device->state();
    const bool optionalSpdif = s && (m_presence & kPresenceSpdifInputs) && (s->spdif_inputs_enabled & 0x0E);
    switch (value) {
    case SourceUsb: return tr("USB");
    case SourceSpdif: return optionalSpdif ? tr("S/PDIF 1") : tr("S/PDIF");
    case SourceI2s: return tr("I2S");
    case SourceAdat: return tr("ADAT");
    case SourceSpdif2:
    case SourceSpdif2 + 1:
    case SourceSpdif2 + 2: return tr("S/PDIF %1").arg(value - SourceSpdif2 + 2);
    default: return tr("Source %1").arg(value);
    }
}

void SidebarController::renameSource(int value, const QString &name) {
    if (!m_sourceNamesSupported || value < 0 || value > SourceLast) return;
    const QByteArray clean = sanitizeSourceName(name.trimmed()).toUtf8();
    if (QString::fromUtf8(clean) == sourceName(value)) return;
    apply(QStringLiteral("input source name %1").arg(value),
          [value, clean](FfiCore *c) { return dspi_set_input_source_name(c, uint8_t(value), clean.constData()); });
}

// ---- Presets -------------------------------------------------------------------

bool SidebarController::presetOccupied(int slot) const {
    const DspState *s = m_ctx.device->state();
    return s && slot >= 0 && slot < MAX_PRESETS && (s->preset_occupied >> slot & 1);
}

QString SidebarController::presetTitle(int slot) const {
    if (!presetOccupied(slot)) return tr("Empty");
    const QString name = fieldText(m_ctx.device->state()->preset_names[slot], CHANNEL_NAME_LEN).trimmed();
    return name.isEmpty() ? tr("Preset %1").arg(slot + 1) : name;
}

QString SidebarController::presetLabel(int slot) const { return QStringLiteral("%1: %2").arg(slot + 1).arg(presetTitle(slot)); }

// The popup's rows; the active one carries "*" while it has unsaved changes
QStringList SidebarController::presetTitles() const {
    QStringList titles;
    if (!m_ctx.device->state()) return titles;
    const int active = m_ctx.device->presets()->activeSlot();
    for (int slot = 0; slot < MAX_PRESETS; ++slot)
        titles.append(slot == active && m_unsaved ? presetTitle(slot) + QLatin1Char('*') : presetTitle(slot));
    return titles;
}

QString SidebarController::unsavedSummary() const {
    QStringList lines;
    for (int i = 0; i < m_unsavedLines.size() && i < kSummaryLines; ++i) lines.append(QStringLiteral("• ") + m_unsavedLines.at(i));
    const int more = int(m_unsavedLines.size()) - kSummaryLines;
    // -- WP QPI2 -- spelt out: without a .qm "%n ... (s)" shows the "(s)"
    if (more == 1) lines.append(tr("…and 1 more change"));
    else if (more > 1) lines.append(tr("…and %1 more changes").arg(more));
    return lines.join(QLatin1Char('\n'));
}

QString SidebarController::presetFailure(uint8_t status) {
    return status == kPresetLocked ? tr("Another console is applying changes") : tr("error %1").arg(status);
}

void SidebarController::setPresetBusy(bool busy) {
    if (busy == m_presetBusy) return;
    m_presetBusy = busy;
    emit presetBusyChanged();
}

// A preset operation through the write path: `op` returns a PRESET_* status
// and a failure shows `failText` with "%1" filled from it
void SidebarController::runPreset(const QString &what, std::function<uint8_t(FfiCore *)> op, const QString &failTitle,
                                  const QString &failText) {
    setPresetBusy(true);
    auto status = std::make_shared<uint8_t>(0xFF);
    apply(
        what,
        [op, status](FfiCore *c) {
            *status = op(c);
            return *status == PRESET_OK;
        },
        [this, status, failTitle, failText](bool ok) {
            setPresetBusy(false);
            if (!ok && !failTitle.isEmpty()) emit presetFailed(failTitle, failText.arg(presetFailure(*status)));
        });
}

// Asks the core, after every write queued so far, whether anything is unsaved
void SidebarController::requestPresetSwitch(int slot) {
    const int active = m_ctx.device->presets()->activeSlot();
    if (slot < 0 || slot >= MAX_PRESETS || slot == active || m_presetBusy || !m_ctx.device->connected()) return;
    setPresetBusy(true);
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            QStringList lines;
            for (const QJsonValue &v :
                 QJsonDocument::fromJson(readSized([c](char *b, uint32_t n) { return dspi_preset_changes(c, b, n); })).array())
                lines.append(v.toString());
            return std::make_pair(dspi_preset_dirty(c), lines);
        },
        [this, slot](std::pair<bool, QStringList> r) {
            setPresetBusy(false);
            m_unsavedLines = r.second;
            m_unsaved = r.first;
            emit unsavedChanged();
            if (r.first) emit unsavedQuestion(QStringLiteral("switch"), slot, unsavedSummary());
            else loadSlot(slot);
        });
}

void SidebarController::requestPresetCopy(int destination) {
    const int active = m_ctx.device->presets()->activeSlot();
    if (destination < 0 || destination >= MAX_PRESETS || destination == active || active < 0 || m_presetBusy) return;
    if (m_unsaved) emit unsavedQuestion(QStringLiteral("copy"), destination, unsavedSummary());
    else copySlot(active, destination, QString());
}

void SidebarController::resolveUnsaved(const QString &op, int slot, const QString &answer) {
    const int active = m_ctx.device->presets()->activeSlot();
    if (answer != QLatin1String("accept") && answer != QLatin1String("alternative")) return;
    const bool save = answer == QLatin1String("accept");
    if (op == QLatin1String("switch")) {
        if (save) saveThenLoad(active, slot);
        else loadSlot(slot);
    } else if (op == QLatin1String("copy") && active >= 0) {
        copySlot(active, slot, save ? QStringLiteral("save") : QStringLiteral("discard"));
    }
}

QString SidebarController::loadFailure(uint8_t status) {
    return status == PRESET_ERR_CRC ? tr("Preset data is corrupted.")
                                    : tr("Failed to load preset (%1).").arg(presetFailure(status));
}

void SidebarController::loadSlot(int slot) {
    setPresetBusy(true);
    auto status = std::make_shared<uint8_t>(0xFF);
    apply(
        QStringLiteral("preset load"),
        [slot, status](FfiCore *c) {
            *status = dspi_load_preset(c, uint8_t(slot));
            return *status == PRESET_OK;
        },
        [this, status](bool ok) {
            setPresetBusy(false);
            if (!ok) emit presetFailed(tr("Load Failed"), loadFailure(*status));
        });
}

// A slot without a name gets "Preset N" before it is saved, so the popup
// stops calling it "Empty"
static bool nameIfEmpty(FfiCore *c, int slot) {
    char name[CHANNEL_NAME_LEN] = {};
    if (dspi_get_preset_name(c, uint8_t(slot), name, sizeof name) && name[0] != 0) return true;
    const QByteArray fallback = SidebarController::tr("Preset %1").arg(slot + 1).toUtf8();
    return dspi_set_preset_name(c, uint8_t(slot), fallback.constData());
}

void SidebarController::saveThenLoad(int previous, int slot) {
    if (previous < 0) {
        loadSlot(slot);
        return;
    }
    // 1: the save failed (the switch stops), 2: the load failed
    auto stage = std::make_shared<int>(0);
    auto status = std::make_shared<uint8_t>(PRESET_OK);
    setPresetBusy(true);
    apply(
        QStringLiteral("preset load"),
        [previous, slot, stage, status](FfiCore *c) {
            nameIfEmpty(c, previous);
            *status = Confirm::savePreset(c, uint8_t(previous));   // -- WP P1QT2 -- token
            if (*status != PRESET_OK) {
                *stage = 1;
                return false;
            }
            *status = dspi_load_preset(c, uint8_t(slot));
            if (*status != PRESET_OK) *stage = 2;
            return *status == PRESET_OK;
        },
        [this, stage, status](bool ok) {
            setPresetBusy(false);
            if (ok) return;
            if (*stage == 1)
                emit presetFailed(tr("Save Failed"),
                                  tr("Failed to save preset (%1). Preset switch aborted.").arg(presetFailure(*status)));
            else
                emit presetFailed(tr("Load Failed"), loadFailure(*status));
        });
}

void SidebarController::savePreset() {
    const int slot = m_ctx.device->presets()->activeSlot();
    if (slot < 0 || m_presetBusy) return;
    runPreset(
        QStringLiteral("preset save"),
        [slot](FfiCore *c) {
            nameIfEmpty(c, slot);
            return Confirm::savePreset(c, uint8_t(slot));   // -- WP P1QT2 -- token
        },
        tr("Save Failed"), tr("Failed to save preset (%1)."));
}

// Copies the live state into `destination` (the core saves it there and back
// to the source, so the source stays the active slot). `preOp` first saves
// ("save") or reloads ("discard") the source when it has unsaved changes.
void SidebarController::copySlot(int sourceSlot, int destination, const QString &preOp) {
    const bool save = preOp == QLatin1String("save"), discard = preOp == QLatin1String("discard");
    runPreset(
        QStringLiteral("preset copy"),
        [sourceSlot, destination, save, discard](FfiCore *c) -> uint8_t {
            const uint64_t generation = dspi_connection_generation(c);
            if (save) {
                nameIfEmpty(c, sourceSlot);
                Confirm::savePreset(c, uint8_t(sourceSlot));   // -- WP P1QT2 -- token
            } else if (discard) {
                dspi_load_preset(c, uint8_t(sourceSlot));
            }
            // Another device came in meanwhile: stop
            if (dspi_connection_generation(c) != generation) return PRESET_OK;
            nameIfEmpty(c, destination);
            return Confirm::copyPreset(c, uint8_t(sourceSlot), uint8_t(destination));   // -- WP P1QT2 -- token
        },
        tr("Copy Failed"), tr("Failed to copy preset (%1)."));
}

void SidebarController::renamePreset(int slot, const QString &name) {
    if (slot < 0 || slot >= MAX_PRESETS) return;
    QString trimmed = name.trimmed();
    while (trimmed.toUtf8().size() >= CHANNEL_NAME_LEN) trimmed.chop(1);
    const QByteArray utf8 = trimmed.toUtf8();
    apply(QStringLiteral("preset name %1").arg(slot), [slot, utf8](FfiCore *c) { return dspi_set_preset_name(c, uint8_t(slot), utf8.constData()); });
}

// Boots into this slot ("specified" startup mode, 0)
void SidebarController::setDefaultPreset() {
    const int slot = m_ctx.device->presets()->activeSlot();
    if (slot < 0) return;
    apply(QStringLiteral("preset startup"), [slot](FfiCore *c) { return dspi_set_preset_startup(c, 0, uint8_t(slot)); });
}

// The core waits until the slot is gone and, for the active slot, loads the
// factory defaults
void SidebarController::clearPreset(int slot) {
    if (!presetOccupied(slot) || m_presetBusy) return;
    runPreset(QStringLiteral("preset clear"), [slot](FfiCore *c) { return Confirm::deletePreset(c, uint8_t(slot)); },
              tr("Clear Failed"), tr("Failed to clear preset (%1)."));
}

void SidebarController::clearAllPresets() {
    const DspState *s = m_ctx.device->state();
    if (!s || m_presetBusy) return;
    const uint16_t occupied = s->preset_occupied;
    const int active = m_ctx.device->presets()->activeSlot();
    runPreset(
        QStringLiteral("preset clear"),
        [occupied, active](FfiCore *c) -> uint8_t {
            uint8_t worst = PRESET_OK;
            for (int slot = 0; slot < MAX_PRESETS; ++slot) {
                if (!(occupied >> slot & 1)) continue;
                const uint8_t status = Confirm::deletePreset(c, uint8_t(slot));   // -- WP P1QT2 -- token
                if (status != PRESET_OK) worst = status;
            }
            // The active slot again, now holding the factory defaults
            if (active >= 0) dspi_load_preset(c, uint8_t(active));
            return worst;
        },
        tr("Clear Failed"), tr("Failed to clear preset (%1)."));
}
