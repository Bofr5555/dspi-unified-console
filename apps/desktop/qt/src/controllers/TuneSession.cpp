// -- WP P5QUI -- The auto-tune, one device's (TuneSession.h).
#include "TuneSession.h"

#include "DeviceSession.h"
#include "MeasureController.h"
#include "MeasureSession.h"
#include "NoticeCenter.h" // -- WP REVERTW --
#include "SessionModel.h"
#include "SpeakersController.h"

#include "CoreApi.h"

#include <QCoreApplication>
#include <QJsonArray>
#include <QJsonDocument>
#include <QTimer>
#include <algorithm>
#include <cmath>
#include <memory>

namespace {

// The `measure.tune.keep_unverified` refusal (PLAN-P5 section 1.4 step 11)
const QLatin1String kKeepUnverified("measure.tune.keep_unverified");
constexpr int kOnceBytes = 8 << 20;

QString str(const QJsonObject &o, const char *key) { return o.value(QLatin1String(key)).toString(); }
bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }

// A call that changes something: once, with room for any answer
QByteArray once(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(kOnceBytes, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

QString feedWords(const QJsonObject &o) {
    const QJsonValue in = o.value(QLatin1String("feedInput"));
    if (!in.isDouble()) return QCoreApplication::translate("TuneController", "no programme feed");
    QString s = QCoreApplication::translate("TuneController", "from input %1 at %2 dB")
                    .arg(in.toInt() + 1)
                    .arg(o.value(QLatin1String("feedGainDb")).toDouble(), 0, 'f', 1);
    if (o.value(QLatin1String("feedInverted")).toBool()) s += QCoreApplication::translate("TuneController", ", inverted");
    return s;
}

} // namespace

TuneSession::TuneSession(MeasureSession *measure) : QObject(measure), m_measure(measure) {
    connect(measure->session()->model(), &SessionModel::applied, this, &TuneSession::onSnapshot);
    if (measure->loop()) {
        connect(measure->loop(), &MeasureLoop::tuneState, this, &TuneSession::onTuneState);
        connect(measure, &MeasureSession::resultChanged, this, &TuneSession::onResult);
    }
    // The measure session is whole by then (its outputs)
    QMetaObject::invokeMethod(this, &TuneSession::onSnapshot, Qt::QueuedConnection);
}

void TuneSession::onSnapshot() {
    const QString serial = m_measure->connected() ? m_measure->model()->serial() : QString();
    if (serial == m_serial) return;
    m_serial = serial;
    if (!serial.isEmpty() && m_measure->awake()) reloadLayout();   // -- WP LAY5b -- else at its wake()
}

// -- WP MDQT2c -- The guard ends with the connection: without Keep it goes back
// by itself, so it goes back now
void TuneSession::stopForClose() {
    disarm();
    if (!applied() || !m_measure->session()) return;
    m_measure->session()->worker()->post([](FfiCore *c) { once([&](char *b, uint32_t l) { return dspi_measure_tune_revert(c, b, l); }); });
    m_measure->session()->drainFirst();
}

void TuneSession::track(int delta) {
    const bool was = busy();
    m_busy = std::max(0, m_busy + delta);
    if (was != busy()) emit busyChanged();
}

void TuneSession::fail(const QString &text) {
    m_error = text;
    emit errorChanged();
}

void TuneSession::clearError() {
    m_error.clear();
    emit errorChanged();
}

// ---- The state -------------------------------------------------------------------------

QString TuneSession::stage() const { return m_state.value(QLatin1String("stage")).toString(QStringLiteral("idle")); }

QString TuneSession::stageText() const {
    const QString s = stage();
    if (s == QLatin1String("proposed")) return tr("Proposed: nothing is written yet.");
    if (s == QLatin1String("sumCheck")) return tr("The summed check plays: both ways of each transition at once.");
    if (s == QLatin1String("applied")) return tr("Applied inside the guard: verify it, then Keep or Revert.");
    if (s == QLatin1String("verifying")) return tr("Verifying inside the guard: the tuned outputs, then the sums.");
    if (s == QLatin1String("verified")) return tr("Verified inside the guard: Keep or Revert.");
    if (s == QLatin1String("kept")) return tr("Kept.");
    if (s == QLatin1String("reverted")) {
        // -- WP P5FOLLOW -- (review P5-E W2, as the web) the core's stage is
        // "reverted" for every guard end but kept; only the guard knows
        // whether the settings from before are really back
        const QJsonObject g = m_state.value(QLatin1String("guard")).toObject();
        const QString phase = str(g, "phase");
        const QString message = str(g, "message");
        const QString tail = message.isEmpty() ? QString() : QLatin1Char(' ') + message;
        if (phase == QLatin1String("lost"))
            return tr("The connection was lost while the tune waited in its guard: the device may still hold the tune.") + tail;
        if (phase == QLatin1String("revert_failed"))
            return tr("The revert failed: the device may still hold part of the tune.") + tail;
        return tr("Reverted: the device is as it was before the tune.");
    }
    // -- WP REVERTW -- the guard did not put everything back: the notice's title and action
    if (s == QLatin1String("revertIncomplete")) {
        const QString key = str(m_state.value(QLatin1String("issue")).toObject(), "key");
        if (auto *notices = Registrar::instance<NoticeCenter>()) return notices->revertText(key.isEmpty() ? QStringLiteral("revert.unknown") : key);
        return tr("The revert was not fully successful. Check the settings, or restore a backup (a snapshot).");
    }
    return tr("No tune yet: measure, then propose one.");
}

QString TuneSession::runText() const {
    if (!running()) return {};
    return tr("Run %1 of %2.").arg(std::max(1, m_state.value(QLatin1String("run")).toInt())).arg(std::max(1, m_state.value(QLatin1String("runs")).toInt()));
}

int TuneSession::progress() const {
    const double p = m_state.value(QLatin1String("session")).toObject().value(QLatin1String("progress")).toDouble();
    return std::isfinite(p) ? int(std::lround(std::clamp(p, 0.0, 1.0) * 100)) : 0;
}

QString TuneSession::issueText() const {
    MeasureSession *m = measure();
    const QJsonObject i = m_state.value(QLatin1String("issue")).toObject();
    if (i.isEmpty() || !m) return {};
    // The keep question has its own words
    if (str(i, "key") == kKeepUnverified) return {};
    return m->issueWords(i);
}

bool TuneSession::applied() const {
    const QString s = stage();
    return s == QLatin1String("applied") || s == QLatin1String("verifying") || s == QLatin1String("verified");
}

int TuneSession::guardRemainingS() const {
    return int(std::ceil(m_state.value(QLatin1String("guard")).toObject().value(QLatin1String("remainingMs")).toDouble() / 1000.0));
}

QString TuneSession::guardText() const {
    const QJsonObject g = m_state.value(QLatin1String("guard")).toObject();
    if (str(g, "phase") == QLatin1String("waiting"))
        return tr("Keep or Revert within %n s; without Keep it goes back by itself.", nullptr, guardRemainingS());
    return str(g, "message");
}

QString TuneSession::verifyText() const {
    const QJsonObject v = m_state.value(QLatin1String("verify")).toObject();
    if (v.isEmpty()) return {};
    QStringList parts;
    parts << (v.value(QLatin1String("passed")).toBool()
                  ? tr("The verify confirmed the tune: measured and predicted agree.")
                  : tr("The verify did not confirm the tune: measured and predicted differ. Revert it, unless you are sure."));
    for (const QJsonValue &n : v.value(QLatin1String("notes")).toArray()) parts << n.toString();
    return parts.join(QStringLiteral(" "));
}

bool TuneSession::verifyPassed() const {
    return m_state.value(QLatin1String("verify")).toObject().value(QLatin1String("passed")).toBool();
}

void TuneSession::onTuneState(const QByteArray &json) {
    const QJsonObject o = MeasureSession::object(json);
    if (o.isEmpty() || !isOk(o)) return;
    const bool wasRunning = running();
    const QString was = stage();
    m_state = o;
    emit stateChanged();
    // -- WP MDQT2c -- the device's tab is busy while the guard waits; a tune on a
    // device that is not shown says how a step of it ended
    updateJob();
    if (was != stage() && !running() && !m_measure->isActive())
        m_measure->owner()->report(m_measure, stageText(), stage() == QLatin1String("reverted") || stage() == QLatin1String("revertIncomplete"));
    // The summed check ended: its verdict is in the proposal
    if (wasRunning && !running()) {
        endRuns();
        if (was == QLatin1String("sumCheck") || stage() == QLatin1String("proposed")) {
            readProposal();
            emit stepDone(QStringLiteral("sumCheck"), stage() == QLatin1String("proposed"));
        } else {
            emit stepDone(QStringLiteral("verify"), stage() == QLatin1String("verified"));
        }
    }
    // The guard ended other than kept (its timeout): a question is moot
    if (!applied() && (m_keepConfirm || m_armed)) disarm();
}

// -- WP MDQT2c --
void TuneSession::updateJob() {
    if (DeviceSession *s = m_measure->session())
        s->setJob(QStringLiteral("TuneController"), applied() ? tr("An auto-tune waits for Keep or Revert.") : QString());
}

void TuneSession::endRuns() {
    if (!m_runs) return;
    m_runs = false;
    if (MeasureSession *m = measure()) {
        m->m_tuneRunning = false;
        if (m->loop()) m->loop()->session.store(false);
        m->closeMic();
    }
}

// ---- The layout ------------------------------------------------------------------------

// The layout tunes what the measurement measured (an output it lacks is refused)
bool TuneSession::measured(int output) const {
    MeasureSession *m = measure();
    const QVariantList outs = m ? m->resultOutputs() : QVariantList();
    if (outs.isEmpty()) return true;
    for (const QVariant &v : outs)
        if (v.toMap().value(QStringLiteral("output")).toInt() == output) return true;
    return false;
}

void TuneSession::onResult() {
    bool changed = false;
    for (QVariant &v : m_layout) {
        QVariantMap row = v.toMap();
        const bool on = measured(row.value(QStringLiteral("output")).toInt());
        if (row.value(QStringLiteral("on")).toBool() == on) continue;
        row.insert(QStringLiteral("on"), on);
        v = row;
        changed = true;
    }
    if (changed) emit layoutChanged();
}

void TuneSession::reloadLayout() {
    MeasureSession *m = measure();
    if (!m) return;
    m->onLoop(this,
              [](FfiCore *c) {
                  // -- WP ROLEQ -- the proposal's feeds with the roles,
                  // speakers and groups of the stored speaker layout
                  const QByteArray proposal = MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_layout_proposal(c, b, l); });
                  const QJsonObject stored = MeasureSession::object(
                      MeasureSession::reply([&](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); }));
                  QJsonObject o = MeasureSession::object(proposal);
                  if (!isOk(o) || !stored.value(QLatin1String("outputs")).isArray()) return proposal;
                  QJsonArray outs;
                  for (const QJsonValue &v : o.value(QLatin1String("outputs")).toArray()) {
                      QJsonObject p = v.toObject();
                      for (const QJsonValue &x : stored.value(QLatin1String("outputs")).toArray()) {
                          const QJsonObject l = x.toObject();
                          if (l.value(QLatin1String("output")) != p.value(QLatin1String("output"))) continue;
                          for (const char *k : { "role", "speaker", "group" }) p.insert(QLatin1String(k), l.value(QLatin1String(k)));
                      }
                      outs.append(p);
                  }
                  o.insert(QStringLiteral("outputs"), outs);
                  return QJsonDocument(o).toJson(QJsonDocument::Compact);
              },
              [this, m](const QByteArray &r) {
                  const QJsonObject o = MeasureSession::object(r);
                  if (!isOk(o)) {
                      m_layout.clear();
                      m_layoutNote = MeasureSession::failureText(o);
                      emit layoutChanged();
                      return;
                  }
                  QVariantList rows;
                  for (const QJsonValue &v : o.value(QLatin1String("outputs")).toArray()) {
                      const QJsonObject p = v.toObject();
                      const int n = p.value(QLatin1String("output")).toInt();
                      rows.append(QVariantMap{ { QStringLiteral("output"), n },
                                               { QStringLiteral("label"), m->outputName(n) },
                                               { QStringLiteral("on"), measured(n) },
                                               { QStringLiteral("role"), str(p, "role") },
                                               { QStringLiteral("speaker"), str(p, "speaker") },
                                               { QStringLiteral("group"), p.value(QLatin1String("group")).toInt() },
                                               { QStringLiteral("name"), str(p, "name") },
                                               { QStringLiteral("feed"), feedWords(p) },
                                               { QStringLiteral("feedInput"), p.value(QLatin1String("feedInput")).toVariant() },
                                               { QStringLiteral("feedGainDb"), p.value(QLatin1String("feedGainDb")).toDouble() },
                                               { QStringLiteral("feedInverted"), p.value(QLatin1String("feedInverted")).toBool() } });
                  }
                  m_layout = rows;
                  QStringList notes;
                  if (!str(o, "template").isEmpty()) notes << tr("Recognised as the template \"%1\".").arg(str(o, "template"));
                  for (const QJsonValue &n : o.value(QLatin1String("notes")).toArray()) notes << n.toString();
                  m_layoutNote = notes.join(QStringLiteral(" "));
                  emit layoutChanged();
              });
}

bool TuneSession::setOutputField(int output, const QString &field, const QVariant &value) {
    for (QVariant &v : m_layout) {
        QVariantMap m = v.toMap();
        if (m.value(QStringLiteral("output")).toInt() != output || m.value(field) == value) continue;
        m.insert(field, value);
        v = m;
        emit layoutChanged();
        return true;
    }
    return false;
}

void TuneSession::setOutputOn(int output, bool on) { setOutputField(output, QStringLiteral("on"), on); }
// -- WP ROLEQ -- a role, speaker or group is written back to the speaker
// layout, its confirmation as it was (only Confirm confirms)
void TuneSession::setOutputRole(int output, const QString &role) {
    if (!setOutputField(output, QStringLiteral("role"), role)) return;
    if (auto *sc = Registrar::instance<SpeakersController>()) sc->writeBack(output, role);
}
void TuneSession::setOutputSpeaker(int output, const QString &speaker) {
    const QString name = speaker.trimmed().left(16);
    if (!setOutputField(output, QStringLiteral("speaker"), name) || name.isEmpty()) return;
    if (auto *sc = Registrar::instance<SpeakersController>()) sc->writeBack(output, QString(), name);
}
void TuneSession::setOutputGroup(int output, int group) {
    group = std::clamp(group, 0, 15);
    if (!setOutputField(output, QStringLiteral("group"), group)) return;
    if (auto *sc = Registrar::instance<SpeakersController>()) sc->writeBack(output, QString(), QString(), group);
}

void TuneSession::setKeepCrossover(bool on) {
    if (on == m_keepCrossover) return;
    m_keepCrossover = on;
    emit optionsChanged();
}
void TuneSession::setKeepDelays(bool on) {
    if (on == m_keepDelays) return;
    m_keepDelays = on;
    emit optionsChanged();
}
void TuneSession::setKeepLevels(bool on) {
    if (on == m_keepLevels) return;
    m_keepLevels = on;
    emit optionsChanged();
}
void TuneSession::setKeepEq(bool on) {
    if (on == m_keepEq) return;
    m_keepEq = on;
    emit optionsChanged();
}

QByteArray TuneSession::layoutJson() const {
    QJsonArray outs;
    for (const QVariant &v : m_layout) {
        const QVariantMap m = v.toMap();
        if (!m.value(QStringLiteral("on")).toBool()) continue;
        QJsonObject o{ { QStringLiteral("output"), m.value(QStringLiteral("output")).toInt() },
                       { QStringLiteral("role"), m.value(QStringLiteral("role")).toString() },
                       { QStringLiteral("speaker"), m.value(QStringLiteral("speaker")).toString() },
                       { QStringLiteral("group"), m.value(QStringLiteral("group")).toInt() },
                       { QStringLiteral("name"), m.value(QStringLiteral("name")).toString() },
                       { QStringLiteral("feedGainDb"), m.value(QStringLiteral("feedGainDb")).toDouble() },
                       { QStringLiteral("feedInverted"), m.value(QStringLiteral("feedInverted")).toBool() } };
        if (m.value(QStringLiteral("feedInput")).isValid() && !m.value(QStringLiteral("feedInput")).isNull())
            o.insert(QStringLiteral("feedInput"), m.value(QStringLiteral("feedInput")).toInt());
        outs.append(o);
    }
    return QJsonDocument(QJsonObject{ { QStringLiteral("outputs"), outs } }).toJson(QJsonDocument::Compact);
}

QByteArray TuneSession::optionsJson() const {
    MeasureSession *m = measure();
    QJsonObject o{ { QStringLiteral("keepCrossover"), m_keepCrossover },
                   { QStringLiteral("keepDelays"), m_keepDelays },
                   { QStringLiteral("keepLevels"), m_keepLevels },
                   { QStringLiteral("keepEq"), m_keepEq } };
    if (m) {
        o.insert(QStringLiteral("targetDocument"), QString::fromUtf8(m->targetDocument()));
        o.insert(QStringLiteral("micDocument"), QString::fromUtf8(m->micDocument()));
    }
    return QJsonDocument(o).toJson(QJsonDocument::Compact);
}

// ---- The proposal ----------------------------------------------------------------------

void TuneSession::propose() {
    clearError();
    MeasureSession *m = measure();
    if (!m) return;
    const QByteArray doc = m->resultDocument();
    if (doc.isEmpty()) {
        fail(tr("No measurement: measure on the Measure tab, or open one on Results or Documents."));
        emit stepDone(QStringLiteral("propose"), false);
        return;
    }
    const QByteArray layout = layoutJson(), options = optionsJson();
    m->onLoop(this,
              [doc, layout, options](FfiCore *c) {
                  return once([&](char *b, uint32_t l) {
                      return dspi_measure_tune_propose(c, doc.constData(), layout.constData(), options.constData(), b, l);
                  });
              },
              [this, m](const QByteArray &r) {
                  const QJsonObject o = MeasureSession::object(r);
                  if (!isOk(o)) {
                      fail(MeasureSession::failureText(o));
                      emit stepDone(QStringLiteral("propose"), false);
                      return;
                  }
                  setProposal(o);
                  // The stage is the core's
                  m->onLoop(this, [](FfiCore *c) { return MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_tune_state(c, b, l); }, 1 << 18); },
                            [this](const QByteArray &st) {
                                onTuneState(st);
                                emit stepDone(QStringLiteral("propose"), true);
                            });
              });
}

void TuneSession::readProposal() {
    MeasureSession *m = measure();
    if (!m) return;
    m->onLoop(this, [](FfiCore *c) { return MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_tune_proposal(c, b, l); }, 1 << 20); },
              [this](const QByteArray &r) {
                  const QJsonObject o = MeasureSession::object(r);
                  if (!isOk(o)) return;
                  if (o.contains(QLatin1String("value")) && o.value(QLatin1String("value")).isNull()) return;
                  setProposal(o);
              });
}

void TuneSession::setProposal(const QJsonObject &p) {
    MeasureSession *m = measure();
    m_proposal = p;
    m_writes.clear();
    for (const QJsonValue &v : p.value(QLatin1String("writes")).toArray()) {
        const QJsonObject w = v.toObject();
        m_writes.append(QVariantMap{ { QStringLiteral("words"), str(w, "words") },
                                     { QStringLiteral("timing"), w.value(QLatin1String("timing")).toBool() },
                                     { QStringLiteral("kind"), str(w, "kind") },
                                     { QStringLiteral("output"), w.value(QLatin1String("output")).toInt() } });
    }
    m_held.clear();
    for (const QJsonValue &v : p.value(QLatin1String("held")).toArray()) m_held << v.toString();
    m_withdrawn.clear();
    for (const QJsonValue &v : p.value(QLatin1String("withdrawn")).toArray())
        m_withdrawn << (m ? m->issueWords(v.toObject()) : str(v.toObject(), "message"));
    m_warnings.clear();
    for (const QJsonValue &v : p.value(QLatin1String("warnings")).toArray())
        m_warnings << (m ? m->issueWords(v.toObject()) : str(v.toObject(), "message"));
    m_notes.clear();
    for (const QJsonValue &v : p.value(QLatin1String("notes")).toArray()) m_notes << v.toString();
    // Each speaker's curves: before solid, predicted dotted, target dashed
    QVariantList list;
    int colour = 0;
    bool spl = false;
    for (const QJsonValue &v : p.value(QLatin1String("curves")).toArray()) {
        const QJsonObject c = v.toObject();
        const QString who = str(c, "speaker").isEmpty() ? tr("Speaker") : tr("Speaker %1").arg(str(c, "speaker"));
        const QJsonObject before = c.value(QLatin1String("before")).toObject();
        spl = spl || before.value(QLatin1String("spl")).toBool();
        list.append(MeasureSession::series(before, QStringLiteral("measured"), tr("%1 before").arg(who), colour));
        list.append(MeasureSession::series(c.value(QLatin1String("predicted")).toObject(), QStringLiteral("predicted"), tr("%1 predicted").arg(who), colour));
        if (colour == 0)
            list.append(MeasureSession::series(c.value(QLatin1String("target")).toObject(), QStringLiteral("target"), tr("Target"), colour));
        ++colour;
    }
    m_graph = MeasureSession::graph(list, spl ? tr("dB SPL") : tr("dB (relative)"));
    emit proposalChanged();
}

QString TuneSession::sumCheckText() const {
    const QString s = sumCheck();
    if (s == QLatin1String("pending"))
        return tr("The summed check has not run: the delay, polarity and crossover changes are applied only after it passed.");
    if (s == QLatin1String("passed")) return tr("The summed check passed.");
    if (s == QLatin1String("failed")) return tr("The summed check failed: the timing changes it did not confirm are withdrawn.");
    if (hasProposal()) return tr("No timing change: nothing needs the summed check.");
    return {};
}

bool TuneSession::needsSumCheck() const { return sumCheck() == QLatin1String("pending"); }

// ---- The runs --------------------------------------------------------------------------

void TuneSession::startRuns(const QString &step, std::function<QByteArray(FfiCore *)> job) {
    clearError();
    MeasureSession *m = measure();
    if (!m || !m->loop()) return;
    if (!m->playBlocker().isEmpty()) {
        fail(m->playBlocker());
        emit stepDone(step, false);
        return;
    }
    if (m->measuring() || m->liveActive() || m->passiveActive()) {   // -- WP P5QUI2 -- the passive RTA too
        fail(tr("Stop the measurement or the live view first."));
        emit stepDone(step, false);
        return;
    }
    m->m_starting = true;   // -- WP MDQT2c --
    m->openMic([this, m, step, job](bool ok, int) {
        if (!ok) {
            m->m_starting = false;
            m->closeMic();
            emit stepDone(step, false);
            return;
        }
        m->onLoop(this, job, [this, m, step](const QByteArray &r) {
            m->m_starting = false;
            const QJsonObject o = MeasureSession::object(r);
            if (!isOk(o)) {
                m->closeMic();
                fail(MeasureSession::failureText(o));
                emit stepDone(step, false);
                return;
            }
            m_runs = true;
            m->m_tuneRunning = true;
            m->m_tuneSession = true;
            m->loop()->tune.store(true);
            m->loop()->session.store(true);
            m->setRunning();
            onTuneState(r);
        });
    });
}

void TuneSession::sumCheckRun() {
    const uint32_t id = uint32_t(proposalId());
    if (!hasProposal()) return;
    startRuns(QStringLiteral("sumCheck"), [id](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_tune_sum_check(c, id, b, l); }); });
}

void TuneSession::verify() {
    // A new verify asks again (the core's rule)
    disarm();
    startRuns(QStringLiteral("verify"), [](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_tune_verify(c, b, l); }); });
}

void TuneSession::stop() {
    MeasureSession *m = measure();
    if (!m) return;
    m->onLoop(this, [](FfiCore *c) { return MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_tune_stop(c, b, l); }, 1 << 18); },
              [this](const QByteArray &r) {
                  endRuns();
                  onTuneState(r);
                  if (stage() == QLatin1String("idle")) {
                      m_proposal = {};
                      m_writes.clear();
                      m_graph.clear();
                      emit proposalChanged();
                  }
              });
}

// ---- Writes ----------------------------------------------------------------------------

void TuneSession::deviceWrite(const QString &step, std::function<QByteArray(FfiCore *)> job,
                                 std::function<void(const QJsonObject &)> done) {
    if (!m_measure->model()) return;   // -- WP MDQT2c --
    auto result = std::make_shared<QByteArray>();
    track(1);
    m_measure->model()->write(
        QStringLiteral("measure tune ") + step,
        [job, result](FfiCore *c) {
            *result = job(c);
            return isOk(MeasureSession::object(*result));
        },
        [this, step, result, done](bool ok) {
            track(-1);
            const QJsonObject o = MeasureSession::object(*result);
            if (!ok) fail(MeasureSession::failureText(o));
            else if (done) done(o);
            // The tune's stage after it
            if (MeasureSession *m = measure())
                m->onLoop(this, [](FfiCore *c) { return MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_tune_state(c, b, l); }, 1 << 18); },
                          [this, step, ok](const QByteArray &st) {
                              onTuneState(st);
                              emit stepDone(step, ok);
                          });
            else emit stepDone(step, ok);
        });
}

void TuneSession::apply(int guardS) {
    clearError();
    if (!hasProposal()) return;
    const uint32_t id = uint32_t(proposalId());
    const uint32_t g = uint32_t(std::clamp(guardS, 0, 590));
    disarm();
    deviceWrite(QStringLiteral("apply"), [id, g](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_tune_apply(c, id, g, b, l); }); },
                [this](const QJsonObject &) {
                    m_keptText.clear();
                    // The guard's timer is the tune tick's
                    if (MeasureSession *m = measure(); m && m->loop()) m->loop()->tune.store(true);
                });
}

void TuneSession::revert() {
    disarm();
    deviceWrite(QStringLiteral("revert"), [](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_tune_revert(c, b, l); }); },
                [this](const QJsonObject &) { endRuns(); });
}

void TuneSession::keep() {
    // After a refusal the person answers the question first, every time
    if (m_armed) {
        if (!m_keepConfirm) {
            m_keepConfirm = true;
            emit keepConfirmChanged();
        }
        return;
    }
    keepNow();
}

void TuneSession::disarm() {
    m_armed = false;
    // Review P5-D: a "Keep Anyway" still waiting for the core's second is
    // withdrawn with it (a Revert pressed meanwhile is the last word)
    ++m_keepToken;
    if (!m_keepConfirm && m_keepConfirmText.isEmpty()) return;
    m_keepConfirm = false;
    m_keepConfirmText.clear();
    emit keepConfirmChanged();
}

void TuneSession::keepNow() {
    clearError();
    MeasureSession *m = measure();
    if (!m) return;
    m->onLoop(this,
              [](FfiCore *c) {
                  // Keep's reply and the state after it, one JSON array
                  const QByteArray k = once([&](char *b, uint32_t l) { return dspi_measure_tune_keep(c, b, l); });
                  const QByteArray st = MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_tune_state(c, b, l); }, 1 << 18);
                  return QByteArray("[") + (k.isEmpty() ? QByteArray("{}") : k) + "," + (st.isEmpty() ? QByteArray("{}") : st) + "]";
              },
              [this](const QByteArray &r) {
                  const QJsonArray both = QJsonDocument::fromJson(r).array();
                  keepAnswered(both.at(0).toObject(), both.at(1).toObject());
              });
}

void TuneSession::keepAnswered(const QJsonObject &reply, const QJsonObject &state) {
    if (!state.isEmpty() && isOk(state)) {
        m_state = state;
        emit stateChanged();
        updateJob();   // -- WP MDQT2c --
    }
    if (isOk(reply)) {
        endRuns();
        disarm();
        QStringList kept{ tr("Kept.") };
        if (!str(reply, "snapshotBeforeId").isEmpty()) kept << tr("\"Before auto-tune\" and \"After auto-tune\" are filed as snapshots: compare them with A/B.");
        for (const QJsonValue &n : reply.value(QLatin1String("notes")).toArray()) kept << n.toString();
        m_keptText = kept.join(QStringLiteral(" "));
        emit stateChanged();
        if (MeasureSession *m = measure()) m->reloadDocuments();
        emit stepDone(QStringLiteral("keep"), true);
        return;
    }
    // After a verify that did not pass: the core asks for a second, explicit
    // Keep. The question is the person's to answer; nothing confirms it here.
    const QJsonObject issue = state.value(QLatin1String("issue")).toObject();
    if (str(issue, "key") == kKeepUnverified || str(reply, "error").contains(kKeepUnverified)) {
        ++m_keepRefusals;
        m_refusedAt.start();
        m_armed = true;
        m_keepConfirm = true;
        m_keepConfirmText = tr("The verify did not confirm this tune: what was measured differs from what was predicted. "
                               "Keep it only if you are sure; Revert puts everything back as it was.");
        emit keepConfirmChanged();
        emit stepDone(QStringLiteral("keep"), false);
        return;
    }
    if (str(issue, "key") == QLatin1String("measure.tune.keep_during_verify")) { emit stepDone(QStringLiteral("keep"), false); return; } // -- WP P5HOLD -- the issue line says it
    fail(MeasureSession::failureText(reply));
    emit stepDone(QStringLiteral("keep"), false);
}

void TuneSession::confirmKeep() {
    // Only the person's answer to the open question
    if (!m_keepConfirm) return;
    m_keepConfirm = false;
    emit keepConfirmChanged();
    // An answer within the core's second is sent when it counts (one press,
    // the person's; nothing is pressed for them)
    const qint64 wait = m_refusedAt.isValid() ? kConfirmDelayMs - m_refusedAt.elapsed() : 0;
    if (wait > 0) {
        // Review P5-D: sent only if nothing withdrew it meanwhile (Revert, a
        // new apply or verify: `disarm`)
        const int token = ++m_keepToken;
        QTimer::singleShot(int(wait), this, [this, token] {
            if (token == m_keepToken) keepNow();
        });
    } else keepNow();
}

void TuneSession::declineKeep() {
    // The guard runs on: Revert, or Keep (which asks again), or its timeout
    if (!m_keepConfirm) return;
    m_keepConfirm = false;
    emit keepConfirmChanged();
}

// ---- A/B -------------------------------------------------------------------------------

QString TuneSession::abText() const {
    if (!abActive()) return {};
    // A is the device with the tune, B the "before" snapshot (the core's)
    return abSide() == QLatin1String("b") ? tr("A/B: the device holds B, as before the tune.")
                                          : tr("A/B: the device holds A, with the tune.");
}

void TuneSession::abStart() {
    deviceWrite(QStringLiteral("ab"), [](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_tune_ab(c, b, l); }); },
                [this](const QJsonObject &o) {
                    m_ab = o;
                    emit abChanged();
                });
}

void TuneSession::abToggle() {
    if (!abActive()) return;
    deviceWrite(QStringLiteral("ab"), [](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_ab_toggle(c, b, l); }); },
                [this](const QJsonObject &o) {
                    m_ab = o;
                    emit abChanged();
                });
}

void TuneSession::abKeep(const QString &side) {
    if (!abActive()) return;
    const QByteArray s = side == QLatin1String("a") ? QByteArray("a") : QByteArray("b");
    deviceWrite(QStringLiteral("ab"), [s](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_ab_keep(c, s.constData(), b, l); }); },
                [this](const QJsonObject &o) {
                    m_ab = o;
                    emit abChanged();
                });
}
