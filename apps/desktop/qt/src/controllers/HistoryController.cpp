#include "HistoryController.h"

#include "ChannelListModel.h"   // -- WP DL3a --
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceSession.h"   // -- WP MDQT2b --
#include "Shell.h"
#include "NoticeCenter.h"
#include "Snapshot.h"

#include <QCoreApplication>
#include <QDateTime>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLocale>
#include <QPointer>
#include <memory>

static const Registrar::Controller<HistoryController> historyController;
static const Registrar::Window historyWindow("edit.history", "HistoryWindow");

namespace {

struct Journal {
    bool canUndo = false;
    bool canRedo = false;
    QJsonArray entries;
};

// The glossary's words for the journal's paths (docs/GLOSSARY.md §2-3); a
// path not listed shows as itself
QString labelOf(const QString &path) {
    static const struct { const char *path; const char *label; } exact[] = {
        { "vol.user", QT_TRANSLATE_NOOP("HistoryController", "Volume") },
        { "vol.master", QT_TRANSLATE_NOOP("HistoryController", "Master volume") },
        { "vol.mute", QT_TRANSLATE_NOOP("HistoryController", "Mute") },
        { "pre", QT_TRANSLATE_NOOP("HistoryController", "Input trim") },
        { "bypass", QT_TRANSLATE_NOOP("HistoryController", "EQ bypass") },
        { "eq.band", QT_TRANSLATE_NOOP("HistoryController", "Filter band") },
        { "eq.type", QT_TRANSLATE_NOOP("HistoryController", "Filter type") },
        { "eq.freq", QT_TRANSLATE_NOOP("HistoryController", "Frequency") },
        { "eq.q", QT_TRANSLATE_NOOP("HistoryController", "Q") },
        { "eq.gain", QT_TRANSLATE_NOOP("HistoryController", "Gain") },
        { "eq.bypass", QT_TRANSLATE_NOOP("HistoryController", "Band bypass") },
        { "ch.delay", QT_TRANSLATE_NOOP("HistoryController", "Delay") },
        { "ch.name", QT_TRANSLATE_NOOP("HistoryController", "Channel name") },
        { "mix", QT_TRANSLATE_NOOP("HistoryController", "Crosspoint") },
        { "out.gain", QT_TRANSLATE_NOOP("HistoryController", "Output gain") },
        { "out.mute", QT_TRANSLATE_NOOP("HistoryController", "Output mute") },
        { "out.delay", QT_TRANSLATE_NOOP("HistoryController", "Delay") },
        { "out.enable", QT_TRANSLATE_NOOP("HistoryController", "Output on") },
        { "in.source", QT_TRANSLATE_NOOP("HistoryController", "Input source") },
        { "dev.name", QT_TRANSLATE_NOOP("HistoryController", "Device name") },
        // -- WP DL3a -- the rest of what the journal keeps
        { "eq.fp", QT_TRANSLATE_NOOP("HistoryController", "Target frequency") },
        { "eq.qp", QT_TRANSLATE_NOOP("HistoryController", "Target Q") },
        { "in.name", QT_TRANSLATE_NOOP("HistoryController", "Source name") },
        { "out.latency", QT_TRANSLATE_NOOP("HistoryController", "Output latency") },
        { "ext.pairmap", QT_TRANSLATE_NOOP("HistoryController", "Crossfeed pairs") },
        { "ext.vreg", QT_TRANSLATE_NOOP("HistoryController", "Core voltage") },
        { "sig.config", QT_TRANSLATE_NOOP("HistoryController", "Signal generator settings") },
        { "bulk", QT_TRANSLATE_NOOP("HistoryController", "All settings written") },
        { "dev.reset", QT_TRANSLATE_NOOP("HistoryController", "Factory reset") },
        { "preset.startup", QT_TRANSLATE_NOOP("HistoryController", "Startup preset") },
        { "preset.iomode", QT_TRANSLATE_NOOP("HistoryController", "I/O storage") },
    };
    // -- WP DL3a -- an effect's parameter: "Loudness › Reference level"; its
    // switch is the effect's name alone
    static const struct { const char *path; const char *label; } param[] = {
        { "limit.threshold", QT_TRANSLATE_NOOP("HistoryController", "Threshold") },
        { "limit.release", QT_TRANSLATE_NOOP("HistoryController", "Release") },
        { "limit.link", QT_TRANSLATE_NOOP("HistoryController", "Link group") },
        { "loud.ref", QT_TRANSLATE_NOOP("HistoryController", "Reference level") },
        { "loud.intensity", QT_TRANSLATE_NOOP("HistoryController", "Intensity") },
        { "loud.mask", QT_TRANSLATE_NOOP("HistoryController", "Outputs") },
        { "lev.amount", QT_TRANSLATE_NOOP("HistoryController", "Amount") },
        { "lev.speed", QT_TRANSLATE_NOOP("HistoryController", "Speed") },
        { "lev.maxgain", QT_TRANSLATE_NOOP("HistoryController", "Maximum boost") },
        { "lev.lookahead", QT_TRANSLATE_NOOP("HistoryController", "Lookahead") },
        { "lev.gate", QT_TRANSLATE_NOOP("HistoryController", "Silence gate") },
        { "lev.masks", QT_TRANSLATE_NOOP("HistoryController", "Channels") },
        { "cf.preset", QT_TRANSLATE_NOOP("HistoryController", "Voicing") },
        { "cf.freq", QT_TRANSLATE_NOOP("HistoryController", "Cutoff frequency") },
        { "cf.feed", QT_TRANSLATE_NOOP("HistoryController", "Feed level") },
        { "cf.itd", QT_TRANSLATE_NOOP("HistoryController", "Interaural delay") },
        { "cf.outputs", QT_TRANSLATE_NOOP("HistoryController", "Outputs") },
        { "bass.cutoff", QT_TRANSLATE_NOOP("HistoryController", "Speaker limit") },
        { "bass.harmonics", QT_TRANSLATE_NOOP("HistoryController", "Harmonics") },
        { "bass.drive", QT_TRANSLATE_NOOP("HistoryController", "Drive") },
        { "bass.character", QT_TRANSLATE_NOOP("HistoryController", "Character") },
        { "bass.original", QT_TRANSLATE_NOOP("HistoryController", "Original bass") },
        { "bass.mask", QT_TRANSLATE_NOOP("HistoryController", "Outputs") },
        { "up.center_mode", QT_TRANSLATE_NOOP("HistoryController", "Centre mode") },
        { "up.surround_mode", QT_TRANSLATE_NOOP("HistoryController", "Surround mode") },
        { "up.strength", QT_TRANSLATE_NOOP("HistoryController", "Strength") },
        { "up.width", QT_TRANSLATE_NOOP("HistoryController", "Centre width") },
        { "up.threshold", QT_TRANSLATE_NOOP("HistoryController", "Correlation threshold") },
        { "up.attack", QT_TRANSLATE_NOOP("HistoryController", "Attack") },
        { "up.release", QT_TRANSLATE_NOOP("HistoryController", "Release") },
        { "up.det_hpf", QT_TRANSLATE_NOOP("HistoryController", "Detector high-pass") },
        { "up.sur_delay", QT_TRANSLATE_NOOP("HistoryController", "Surround delay") },
        { "up.sur_hpf", QT_TRANSLATE_NOOP("HistoryController", "Surround high-pass") },
        { "up.sur_lpf", QT_TRANSLATE_NOOP("HistoryController", "Surround low-pass") },
        { "up.decorr", QT_TRANSLATE_NOOP("HistoryController", "Decorrelation") },
        { "up.presence", QT_TRANSLATE_NOOP("HistoryController", "Centre presence") },
        { "up.config", QT_TRANSLATE_NOOP("HistoryController", "Settings") },
        { "sub.low", QT_TRANSLATE_NOOP("HistoryController", "24 - 36 Hz") },
        { "sub.high", QT_TRANSLATE_NOOP("HistoryController", "36 - 56 Hz") },
        { "sub.top", QT_TRANSLATE_NOOP("HistoryController", "56 - 80 Hz") },
        { "sub.boost", QT_TRANSLATE_NOOP("HistoryController", "70 Hz bell") },
        { "sub.mask", QT_TRANSLATE_NOOP("HistoryController", "Outputs") },
        { "sub.select", QT_TRANSLATE_NOOP("HistoryController", "Selectivity") },
        { "sub.depth", QT_TRANSLATE_NOOP("HistoryController", "Depth") },
        { "sub.hold", QT_TRANSLATE_NOOP("HistoryController", "Hold") },
        { "sub.ceiling", QT_TRANSLATE_NOOP("HistoryController", "Sub ceiling") },
        { "sub.link", QT_TRANSLATE_NOOP("HistoryController", "Link output pairs") },
        { "sub.solo", QT_TRANSLATE_NOOP("HistoryController", "Solo") },
        { "tube.mask", QT_TRANSLATE_NOOP("HistoryController", "Outputs") },
        { "tube.type", QT_TRANSLATE_NOOP("HistoryController", "Tube") },
        { "tube.drive", QT_TRANSLATE_NOOP("HistoryController", "Drive") },
        { "tube.bias", QT_TRANSLATE_NOOP("HistoryController", "Bias") },
        { "tube.asym", QT_TRANSLATE_NOOP("HistoryController", "Asymmetry") },
        { "tube.hardness", QT_TRANSLATE_NOOP("HistoryController", "Knee hardness") },
        { "tube.sag", QT_TRANSLATE_NOOP("HistoryController", "Sag") },
        { "tube.rectifier", QT_TRANSLATE_NOOP("HistoryController", "Rectifier") },
        { "tube.xfmr", QT_TRANSLATE_NOOP("HistoryController", "Output stage") },
        { "tube.damping", QT_TRANSLATE_NOOP("HistoryController", "Damping factor") },
        { "tube.resonance", QT_TRANSLATE_NOOP("HistoryController", "Speaker resonance") },
        { "tube.mix", QT_TRANSLATE_NOOP("HistoryController", "Mix") },
        { "tube.trim", QT_TRANSLATE_NOOP("HistoryController", "Output trim") },
    };
    static const struct { const char *prefix; const char *label; } feature[] = {
        { "limit.", QT_TRANSLATE_NOOP("HistoryController", "Output Limiter") },
        { "loud.", QT_TRANSLATE_NOOP("HistoryController", "Loudness") },
        { "lev.", QT_TRANSLATE_NOOP("HistoryController", "Leveller") },
        { "cf.", QT_TRANSLATE_NOOP("HistoryController", "Crossfeed") },
        { "ext.pairmap", QT_TRANSLATE_NOOP("HistoryController", "Crossfeed") },
        { "bass.", QT_TRANSLATE_NOOP("HistoryController", "Psychoacoustic Bass") },
        { "up.", QT_TRANSLATE_NOOP("HistoryController", "Upmixer") },
        { "sub.", QT_TRANSLATE_NOOP("HistoryController", "Subharmonic Synthesizer") },
        { "tube.", QT_TRANSLATE_NOOP("HistoryController", "Tube Modeller") },
        { "preset.", QT_TRANSLATE_NOOP("HistoryController", "Preset") },
        { "cs.", QT_TRANSLATE_NOOP("HistoryController", "Control surface") },
        { "ext.alink.", QT_TRANSLATE_NOOP("HistoryController", "Audio link") },   // -- WP DL3a --
    };
    for (const auto &e : exact)
        if (path == QLatin1String(e.path)) return QCoreApplication::translate("HistoryController", e.label);
    for (const auto &f : feature) {
        if (!path.startsWith(QLatin1String(f.prefix))) continue;
        const QString name = QCoreApplication::translate("HistoryController", f.label);
        for (const auto &p : param)   // -- WP DL3a --
            if (path == QLatin1String(p.path))
                //: An effect's setting in the change history: %1 the effect, %2 the setting
                return QCoreApplication::translate("HistoryController", "%1 › %2")
                    .arg(name, QCoreApplication::translate("HistoryController", p.label));
        return name;
    }
    return path;
}

// The glossary's unit of a path's number, "" for none
QString unitOf(const QString &path) {
    if (path == QLatin1String("eq.freq") || path == QLatin1String("cf.freq")) return QStringLiteral("Hz");
    if (path.endsWith(QLatin1String(".delay")) || path.endsWith(QLatin1String(".release"))) return QStringLiteral("ms");
    if (path == QLatin1String("limit.threshold") || path == QLatin1String("lev.gate")) return QStringLiteral("dBFS");
    if (path == QLatin1String("loud.ref")) return QStringLiteral("dB SPL");
    if (path.startsWith(QLatin1String("vol.")) || path == QLatin1String("pre") || path.endsWith(QLatin1String(".gain"))
        || path == QLatin1String("mix") || path == QLatin1String("cf.feed"))
        return QStringLiteral("dB");
    return QString();
}

// A parameter value ({"float": -7.5}, {"bool": true}, ...) in words
QString valueText(const QJsonValue &v, const QString &path) {
    if (!v.isObject()) return QString();
    const QJsonObject value = v.toObject();
    if (value.contains(QLatin1String("bool")))
        return value.value(QLatin1String("bool")).toBool() ? QCoreApplication::translate("HistoryController", "On")
                                                        : QCoreApplication::translate("HistoryController", "Off");
    if (value.contains(QLatin1String("float"))) {
        const double x = value.value(QLatin1String("float")).toDouble();
        const QString unit = unitOf(path);
        QString number = QLocale().toString(x, 'f', unit == QLatin1String("Hz") ? 0 : (path == QLatin1String("eq.q") ? 2 : 1));
        return unit.isEmpty() ? number : number + QLatin1Char(' ') + unit;
    }
    if (value.contains(QLatin1String("int"))) return QString::number(value.value(QLatin1String("int")).toInteger());
    if (value.contains(QLatin1String("choice"))) return QString::number(value.value(QLatin1String("choice")).toInt());
    if (value.contains(QLatin1String("text"))) return value.value(QLatin1String("text")).toString();
    return QString();
}

} // namespace

HistoryController::HistoryController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    ctx.shell->addHandler(QStringLiteral("edit.undo"), this, [this] { undo(); });
    ctx.shell->addHandler(QStringLiteral("edit.redo"), this, [this] { redo(); });
    // Every snapshot: this app's own writes raise no notification, and a
    // NOTIFY_HISTORY batch ends in one too. DeviceModel::apply emits
    // stateChanged beside it, so one connection reads the journal once.
    connect(ctx.device, &DeviceModel::snapshotApplied, this, [this] { reload(); });
}

HistoryController *HistoryController::create(QQmlEngine *, QJSEngine *) {
    HistoryController *self = Registrar::instance<HistoryController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

QVariantMap HistoryController::describe(const QJsonObject &entry, const ChannelNames *names) {
    const QString path = entry.value(QLatin1String("path")).toString();
    QStringList where;
    const QJsonArray indices = entry.value(QLatin1String("indices")).toArray();
    // -- WP DL3a -- a channel by its name when the device gives one
    const auto named = [names](const QHash<int, QString> ChannelNames::*table, int i, const QString &number) {
        const QString name = names ? (names->*table).value(i) : QString();
        return name.isEmpty() ? number : name;
    };
    // -- WP SHOWFIX -- (finding 5) numbered as the screens' words do, "Out 3"
    const auto output = [&](int i) { return named(&ChannelNames::outputs, i, tr("Out %1").arg(i + 1)); };
    const auto input = [&](int i) { return named(&ChannelNames::inputs, i, tr("In %1").arg(i + 1)); };
    const int first = indices.isEmpty() ? -1 : indices.at(0).toInt();
    // -- WP DL3a -- a crossover band is a band from XOVER_BAND_BASE on
    const bool crossover = path.startsWith(QLatin1String("eq.")) && indices.size() > 1
                           && indices.at(1).toInt() >= XOVER_BAND_BASE;
    QString title = labelOf(path);
    if (path.startsWith(QLatin1String("out.")) && !indices.isEmpty())
        where << output(first);
    else if ((path.startsWith(QLatin1String("eq.")) || path.startsWith(QLatin1String("ch.")) || path == QLatin1String("pre")
              || path == QLatin1String("bypass"))
             && !indices.isEmpty()) {
        //: A wire channel's number in the change history
        where << named(&ChannelNames::wire, first, tr("channel %1").arg(first + 1));
        if (crossover) {
            static const struct { const char *path; const char *label; } xo[] = {
                { "eq.band", QT_TR_NOOP("Crossover filter") },
                { "eq.type", QT_TR_NOOP("Crossover type") },
                { "eq.freq", QT_TR_NOOP("Crossover frequency") },
                { "eq.q", QT_TR_NOOP("Crossover Q") },
                { "eq.gain", QT_TR_NOOP("Crossover gain") },
                { "eq.bypass", QT_TR_NOOP("Crossover bypass") },
            };
            for (const auto &x : xo)
                if (path == QLatin1String(x.path)) title = tr(x.label);
            //: One of a channel's crossover filters (1 to 4) in the change history
            where << tr("filter %1").arg(indices.at(1).toInt() - XOVER_BAND_BASE + 1);
        } else if (path.startsWith(QLatin1String("eq.")) && indices.size() > 1)
            where << tr("band %1").arg(indices.at(1).toInt() + 1);
    } else if (path == QLatin1String("mix") && indices.size() > 1) {
        //: A matrix crosspoint: input %1 to output %2
        where << tr("%1 to %2").arg(input(first), output(indices.at(1).toInt()));
    } else if (path.startsWith(QLatin1String("preset.")) && !indices.isEmpty()) {
        // -- WP DL3a -- a slot as the Presets page numbers it
        const int slot = first + 1;
        if (path == QLatin1String("preset.load")) title = tr("Preset %1 loaded").arg(slot);
        else if (path == QLatin1String("preset.save")) title = tr("Saved to preset %1").arg(slot);
        else if (path == QLatin1String("preset.delete")) title = tr("Preset %1 cleared").arg(slot);
        else if (path == QLatin1String("preset.name")) title = tr("Preset %1 name").arg(slot);
        else where << tr("preset %1").arg(slot);
    }
    // -- WP DL3a -- the generator's start and stop (its choices, SIGGEN_CTL_*)
    if (path == QLatin1String("sig.control")) {
        const int choice = entry.value(QLatin1String("after")).toObject().value(QLatin1String("choice")).toInt(-1);
        title = choice == 1 ? tr("Signal generator started") : choice == 2 ? tr("Signal generator stopped at once")
                                                                            : tr("Signal generator stopped");
    }
    if (!where.isEmpty()) title += QStringLiteral(", ") + where.join(QStringLiteral(", "));
    // -- WP P1QT2 -- the group it was part of (P1UG: Clear All, Direct 1:1, AutoEQ, ...)
    const QString label = entry.value(QLatin1String("label")).toString();
    // -- WP SHOWFIX -- (finding 5) a measurement's generator changes, one row
    // the core words ("Measurement: Out 1–3, 6 sweeps")
    const bool run = path == QLatin1String("sig.run");
    if (run) title = label.isEmpty() ? tr("Measurement") : label;
    //: A change made as part of a group: %1 the group ("Clear All"), %2 the change
    else if (!label.isEmpty()) title = QCoreApplication::translate("HistoryController", "%1: %2").arg(label, title);
    // -- WP DL3a -- the generator's choice is in the title
    const bool worded = path == QLatin1String("sig.control") || run;
    const QString before = worded ? QString() : valueText(entry.value(QLatin1String("before")), path);
    const QString after = worded ? QString() : valueText(entry.value(QLatin1String("after")), path);
    QString detail;
    if (!before.isEmpty() && !after.isEmpty()) detail = before + QStringLiteral(" → ") + after;
    else detail = after;
    return { { QStringLiteral("title"), title }, { QStringLiteral("detail"), detail } };
}

// -- WP DL3a -- the channels' names, read on the GUI thread
HistoryController::ChannelNames HistoryController::channelNames() const {
    ChannelNames names;
    const ChannelListModel *channels = m_ctx.device ? m_ctx.device->channels() : nullptr;
    if (!channels) return names;
    for (int row = 0; row < channels->rowCount(); ++row) {
        const QModelIndex at = channels->index(row);
        // -- WP SHOWFIX -- (finding 5) as the person knows it, the core's word
        // for every text: "Centre woofer" from the speaker layout, "LFE", "Out 3"
        QString label = channels->data(at, ChannelListModel::ShortNameRole).toString();
        if (label.isEmpty()) label = channels->data(at, ChannelListModel::DescriptorRole).toString();
        if (label.isEmpty()) continue;
        names.wire.insert(channels->data(at, ChannelListModel::ChannelRole).toInt(), label);
        const int index = channels->data(at, ChannelListModel::IndexRole).toInt();
        if (channels->data(at, ChannelListModel::KindRole).toString() == QLatin1String("output")) names.outputs.insert(index, label);
        else names.inputs.insert(index, label);
    }
    return names;
}

void HistoryController::reload() {
    if (m_reading) {
        m_again = true;
        return;
    }
    m_reading = true;
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            Journal j;
            j.canUndo = dspi_can_undo(c);
            j.canRedo = dspi_can_redo(c);
            const QByteArray reply = readSized([c](char *buf, uint32_t len) { return dspi_history(c, kShown, buf, len); }, 8192);
            j.entries = jsonValue(reply).toArray();
            return j;
        },
        [this, asked](const Journal &j) {
            m_reading = false;
            // -- WP MDQT2b -- the journal of a device that is not the active
            // one any more: not shown, the active one's is read instead
            if (asked != m_ctx.activeSession()) {
                m_again = false;
                reload();
                return;
            }
            QVariantList list;
            const ChannelNames names = channelNames();   // -- WP DL3a --
            for (const QJsonValue &v : j.entries) {
                const QJsonObject e = v.toObject();
                QVariantMap row = describe(e, &names);
                row.insert(QStringLiteral("source"), e.value(QLatin1String("source")).toString());
                // -- WP MCP4 --
                row.insert(QStringLiteral("agent"), e.value(QLatin1String("actor")).toInteger() != 0
                                                        ? e.value(QLatin1String("actorLabel")).toString(QStringLiteral("AI")) : QString());
                row.insert(QStringLiteral("undoable"), e.value(QLatin1String("undoable")).toBool());
                const qint64 at = e.value(QLatin1String("atMs")).toInteger();
                row.insert(QStringLiteral("time"), QLocale().toString(QDateTime::fromMSecsSinceEpoch(at).time(), QLocale::ShortFormat));
                list.append(row);
            }
            if (j.canUndo != m_canUndo || j.canRedo != m_canRedo || list != m_entries) {
                m_canUndo = j.canUndo;
                m_canRedo = j.canRedo;
                m_entries = list;
                emit changed();
            }
            if (m_again) {
                m_again = false;
                reload();
            }
        });
}

void HistoryController::undo() { step(false); }
void HistoryController::redo() { step(true); }

void HistoryController::step(bool forward) {
    if (!m_ctx.device->connected()) return;
    // Each step is its own write: two quick undos are two steps, not one
    auto reply = std::make_shared<QByteArray>();
    const QString what = (forward ? QStringLiteral("redo %1") : QStringLiteral("undo %1")).arg(++m_steps);
    m_ctx.device->write(
        what,
        [reply, forward](FfiCore *c) {
            *reply = readSized([c, forward](char *buf, uint32_t len) {
                return forward ? dspi_redo(c, buf, len) : dspi_undo(c, buf, len);
            });
            return QJsonDocument::fromJson(*reply).object().value(QLatin1String("ok")).toBool();
        },
        [this, reply](bool ok) {
            const QJsonObject r = QJsonDocument::fromJson(*reply).object();
            const QJsonValue entry = r.value(QLatin1String("entry"));
            const ChannelNames known = channelNames();   // -- WP DL3a --
            const QString title = entry.isObject() ? describe(entry.toObject(), &known).value(QStringLiteral("title")).toString() : QString();
            m_lastNote = r.value(QLatin1String("note")).toString();
            NoticeCenter *notices = Registrar::instance<NoticeCenter>();
            if (notices && !m_lastNote.isEmpty()) notices->post(m_lastNote, QStringLiteral("warning"));
            const QJsonArray skipped = r.value(QLatin1String("skipped")).toArray();
            if (notices && !skipped.isEmpty() && m_lastNote.isEmpty()) {
                QStringList names;
                for (const QJsonValue &s : skipped) names << describe(s.toObject(), &known).value(QStringLiteral("title")).toString();
                notices->post(skipped.size() == 1
                                  //: %1 names a change that cannot be undone (a preset load, an output switched on)
                                  ? tr("Stepped over a change that cannot be undone: %1").arg(names.first())
                                  : tr("Stepped over changes that cannot be undone: %1").arg(names.join(QStringLiteral("; "))),
                              QStringLiteral("info"));
            }
            // The C API's failure reply is {ok:false, code, error}
            if (!ok && notices) {
                const QString error = r.value(QLatin1String("error")).toString();
                if (!error.isEmpty()) notices->post(error, QStringLiteral("error"));
            }
            emit moved(ok, title);
            // A failed step brings no snapshot; a done one reloads through it
            if (!ok) reload();
        });
}
