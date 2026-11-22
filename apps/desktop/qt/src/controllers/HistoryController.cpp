#include "HistoryController.h"

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
    };
    for (const auto &e : exact)
        if (path == QLatin1String(e.path)) return QCoreApplication::translate("HistoryController", e.label);
    for (const auto &f : feature)
        if (path.startsWith(QLatin1String(f.prefix))) return QCoreApplication::translate("HistoryController", f.label);
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

QVariantMap HistoryController::describe(const QJsonObject &entry) {
    const QString path = entry.value(QLatin1String("path")).toString();
    QStringList where;
    const QJsonArray indices = entry.value(QLatin1String("indices")).toArray();
    if (path.startsWith(QLatin1String("out.")) && !indices.isEmpty())
        where << QCoreApplication::translate("HistoryController", "OUT%1").arg(indices.at(0).toInt() + 1);
    else if ((path.startsWith(QLatin1String("eq.")) || path.startsWith(QLatin1String("ch.")) || path == QLatin1String("pre")
              || path == QLatin1String("bypass"))
             && !indices.isEmpty()) {
        //: A wire channel's number in the change history
        where << QCoreApplication::translate("HistoryController", "channel %1").arg(indices.at(0).toInt() + 1);
        if (path.startsWith(QLatin1String("eq.")) && indices.size() > 1)
            where << QCoreApplication::translate("HistoryController", "band %1").arg(indices.at(1).toInt() + 1);
    } else if (path == QLatin1String("mix") && indices.size() > 1) {
        //: A matrix crosspoint: input %1 to output %2
        where << QCoreApplication::translate("HistoryController", "IN%1 to OUT%2").arg(indices.at(0).toInt() + 1).arg(indices.at(1).toInt() + 1);
    }
    QString title = labelOf(path);
    if (!where.isEmpty()) title += QStringLiteral(", ") + where.join(QStringLiteral(", "));
    // -- WP P1QT2 -- the group it was part of (P1UG: Clear All, Direct 1:1, AutoEQ, ...)
    const QString label = entry.value(QLatin1String("label")).toString();
    //: A change made as part of a group: %1 the group ("Clear All"), %2 the change
    if (!label.isEmpty()) title = QCoreApplication::translate("HistoryController", "%1: %2").arg(label, title);
    const QString before = valueText(entry.value(QLatin1String("before")), path);
    const QString after = valueText(entry.value(QLatin1String("after")), path);
    QString detail;
    if (!before.isEmpty() && !after.isEmpty()) detail = before + QStringLiteral(" → ") + after;
    else detail = after;
    return { { QStringLiteral("title"), title }, { QStringLiteral("detail"), detail } };
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
            for (const QJsonValue &v : j.entries) {
                const QJsonObject e = v.toObject();
                QVariantMap row = describe(e);
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
            const QString title = entry.isObject() ? describe(entry.toObject()).value(QStringLiteral("title")).toString() : QString();
            m_lastNote = r.value(QLatin1String("note")).toString();
            NoticeCenter *notices = Registrar::instance<NoticeCenter>();
            if (notices && !m_lastNote.isEmpty()) notices->post(m_lastNote, QStringLiteral("warning"));
            const QJsonArray skipped = r.value(QLatin1String("skipped")).toArray();
            if (notices && !skipped.isEmpty() && m_lastNote.isEmpty()) {
                QStringList names;
                for (const QJsonValue &s : skipped) names << describe(s.toObject()).value(QStringLiteral("title")).toString();
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
