// -- WP QP6 -- The matrix mixer's controller (MatrixController.h).
#include "MatrixController.h"

#include "ChannelController.h"
#include "ChannelListModel.h"
#include "Core.h"
#include "DeviceModel.h"
#include "SessionState.h"

#include <QRegularExpression>
#include <QSettings>
#include <cmath>
#include <utility>
#include <vector>

static const Registrar::Controller<MatrixController> matrixController;
static const Registrar::Window matrixWindow("tools.matrix", "MatrixWindow");

namespace {

// The upmixer's surround engine is off (the Mac's UPMIX_SURROUND_MODE_OFF)
constexpr float SurroundModeOff = 0.0f;

// One crosspoint write of a batch
struct RouteWrite {
    uint8_t input, output;
    bool enabled;
    float gain;
    bool invert;
};

} // namespace

MatrixController::MatrixController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx), m_model(new MatrixModel(this)) {
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &MatrixController::onSnapshot);
    // A refused crosspoint shows the device's value again
    connect(ctx.device, &DeviceModel::writeFailed, this, [this](const QString &what) {
        static const QRegularExpression route(QStringLiteral("^matrix route (\\d+) (\\d+)$"));
        const QRegularExpressionMatch m = route.match(what);
        // -- WP MX1 -- a linked pair's write: the even row's crosspoint and its mirror
        static const QRegularExpression pair(QStringLiteral("^matrix route pair (\\d+) (\\d+)$"));
        const QRegularExpressionMatch p = pair.match(what);
        if (m.hasMatch()) m_model->unstage(m.captured(1).toInt(), m.captured(2).toInt());
        else if (p.hasMatch()) {
            const int i = p.captured(1).toInt(), o = p.captured(2).toInt();
            m_model->unstage(i, o);
            m_model->unstage(i ^ 1, o ^ 1);
        } else if (what == QLatin1String("matrix routes")) m_model->unstageAll();
    });
    // Nothing queued or running: the last snapshot carries every write
    connect(ctx.device, &DeviceModel::busyChanged, this, [this] {
        if (!m_ctx.device->busy()) m_model->unstageAll();
    });
    m_dense = QSettings().value(QStringLiteral("matrix/dense"), false).toBool();   // -- WP MX1 --
    onSnapshot();
}

ChannelController *MatrixController::channels() const { return Registrar::instance<ChannelController>(); }

bool MatrixController::writable() const { return m_ctx.device->connected() && !m_ctx.device->readOnly(); }

int MatrixController::pdmOutput() const { return m_ctx.device->numOutputs() - 1; }

bool MatrixController::wide() const {
    const DspState *s = m_ctx.device->state();
    return s && m_ctx.device->platformName() == QLatin1String("RP2350") && s->format_version == WIRE_FORMAT_VERSION &&
           s->num_input_channels == MAX_INPUTS;
}

bool MatrixController::upmixRows() const {
    const DspState *s = m_ctx.device->state();
    return s && s->format_version >= 26 && m_ctx.device->platformName() == QLatin1String("RP2350") &&
           s->upmix[UPMIX_PARAM_ENABLED] != 0.0f && m_ctx.device->channels()->inputCount() == 2;
}

double MatrixController::maxDelayMs() const {
    const ChannelController *cc = channels();
    return cc ? cc->maxDelayMs() : 21.0;
}

int MatrixController::sourceRows(int inputs, bool upmixDerives, bool surroundOff) {
    if (!upmixDerives) return inputs;
    return surroundOff ? 3 : 5;
}

void MatrixController::onSnapshot() {
    // -- WP MDQT2a -- Another device is the active one: the crosspoints staged
    // for the one left wait in its session; this one's count only while its
    // writes are on their way
    const std::shared_ptr<State> st = sessionState<State>(m_ctx.activeSession(), QStringLiteral("MatrixController"));
    if (st != m_shown) {
        const QHash<int, MatrixModel::Cell> left =
            m_model->swapStaged(m_ctx.device->busy() ? std::exchange(st->staged, {}) : QHash<int, MatrixModel::Cell>());
        if (m_shown) m_shown->staged = left;
        st->staged.clear();
        m_shown = st;
    }
    const DspState *s = m_ctx.device->state();
    const int outputs = s ? m_ctx.device->numOutputs() : 0;
    const bool derive = upmixRows();
    const int rows = s ? sourceRows(m_ctx.device->channels()->inputCount(), derive,
                                    s->upmix[UPMIX_PARAM_SURROUND_MODE] == SurroundModeOff)
                       : 0;
    // The PDM column is outlined while enabling PDM would collide on Core 1
    QVector<bool> conflict(outputs);
    const ChannelController *cc = channels();
    const int pdm = pdmOutput();
    if (cc && pdm >= 0 && pdm < outputs) conflict[pdm] = cc->enableWouldConflict(pdm);
    m_model->update(s, rows, outputs, conflict);
    ++m_revision;
    loadLinkPairs();   // -- WP MX1 --
    if (m_currentInput >= 0 && !m_model->valid(m_currentInput, m_currentOutput)) setCurrent(-1, -1);
    emit changed();
}

// ---- The view (-- WP MX1 --) -------------------------------------------------------

// Link L/R pairs of the device shown: kept under matrix/linkPairs/<serial>
void MatrixController::loadLinkPairs() {
    const QString serial = m_ctx.device->connected() ? m_ctx.device->serial() : QString();
    if (serial == m_linkSerial) return;
    m_linkSerial = serial;
    const bool on = !serial.isEmpty() && QSettings().value(QStringLiteral("matrix/linkPairs/%1").arg(serial), false).toBool();
    if (on == m_linkPairs) return;
    m_linkPairs = on;
    emit viewChanged();
}

void MatrixController::setLinkPairs(bool on) {
    if (on == m_linkPairs) return;
    m_linkPairs = on;
    if (!m_linkSerial.isEmpty()) QSettings().setValue(QStringLiteral("matrix/linkPairs/%1").arg(m_linkSerial), on);
    emit viewChanged();
}

void MatrixController::setDense(bool on) {
    if (on == m_dense) return;
    m_dense = on;
    QSettings().setValue(QStringLiteral("matrix/dense"), on);
    emit viewChanged();
}

void MatrixController::setCurrent(int input, int output) {
    if (!m_model->valid(input, output)) input = output = -1;
    if (input == m_currentInput && output == m_currentOutput) return;
    m_currentInput = input;
    m_currentOutput = output;
    emit currentChanged();
}

int MatrixController::pairRow(int row) const {
    const int inputs = m_ctx.device->channels()->inputCount();
    const int partner = row ^ 1;
    return row >= 0 && row < rows() && partner < rows() && row < inputs && partner < inputs ? partner : -1;
}

QVariantMap MatrixController::mirrorOf(int input, int output) const {
    const int i = pairRow(input), o = output ^ 1;
    if (!m_linkPairs || i < 0 || output < 0 || !m_model->valid(i, o)) return {};
    return { { QStringLiteral("input"), i }, { QStringLiteral("output"), o } };
}

// ---- Reading -------------------------------------------------------------------

QVariantMap MatrixController::row(int row) const {
    const DspState *s = m_ctx.device->state();
    if (!s || row < 0 || row >= rows()) return {};
    if (upmixRows() && row >= 2) {
        static const char *const shortNames[] = { QT_TR_NOOP("C"), QT_TR_NOOP("Ls"), QT_TR_NOOP("Rs") };
        static const char *const fullNames[] = { QT_TR_NOOP("Upmix Centre"), QT_TR_NOOP("Upmix Left Surround"),
                                                 QT_TR_NOOP("Upmix Right Surround") };
        const int d = qBound(0, row - 2, 2);
        return { { QStringLiteral("name"), tr(shortNames[d]) },
                 { QStringLiteral("fullName"), tr(fullNames[d]) },
                 { QStringLiteral("input"), -1 },
                 { QStringLiteral("trim"), 0.0 } };
    }
    const ChannelListModel *list = m_ctx.device->channels();
    const int at = list->rowOfChannel(row);
    QString name = at >= 0 ? list->get(at, QStringLiteral("label")).toString() : QString();
    if (name.isEmpty()) name = tr("USB %1").arg(row + 1);
    return { { QStringLiteral("name"), name },
             { QStringLiteral("fullName"), name },
             { QStringLiteral("input"), row },
             { QStringLiteral("trim"), double(s->input_preamp_db[row]) } };
}

QVariantMap MatrixController::column(int output) const {
    const DspState *s = m_ctx.device->state();
    if (!s || output < 0 || output >= columns()) return {};
    const ChannelListModel *list = m_ctx.device->channels();
    const ChannelController *cc = channels();
    const int channel = cc ? cc->outputChannel(output) : list->inputCount() + output;
    const int at = list->rowOfChannel(channel);
    QString label = at >= 0 ? list->get(at, QStringLiteral("label")).toString() : QString();
    if (label.isEmpty()) label = tr("Out %1").arg(output + 1);
    const QString name = at >= 0 ? list->get(at, QStringLiteral("name")).toString() : label;
    const bool enabled = s->output_enabled[output];
    const bool conflict = !enabled && cc && cc->enableWouldConflict(output);
    const QString tip = enabled    ? tr("Disable output (saves CPU)")
                        : conflict ? tr("Conflict: shared Core 1 resource")
                                   : tr("Enable output");
    return { { QStringLiteral("channel"), channel },
             { QStringLiteral("label"), label },
             { QStringLiteral("name"), name },
             { QStringLiteral("descriptor"), QStringLiteral("OUT%1").arg(output + 1) },
             { QStringLiteral("pdm"), output == pdmOutput() },
             { QStringLiteral("enabled"), enabled },
             { QStringLiteral("muted"), s->output_muted[output] },
             { QStringLiteral("gain"), double(s->output_gain_db[output]) },
             { QStringLiteral("delay"), double(s->output_delay_ms[output]) },
             { QStringLiteral("conflict"), conflict },
             { QStringLiteral("enableTip"), tip } };
}

// ---- Crosspoints ---------------------------------------------------------------

// The cell as shown, edited, staged, then sent through the output card's writer
void MatrixController::editCell(int input, int output, const std::function<void(MatrixModel::Cell &)> &edit) {
    ChannelController *cc = channels();
    if (!writable() || !cc || !m_model->valid(input, output)) return;
    MatrixModel::Cell c = m_model->cell(input, output);
    edit(c);
    m_model->stage(input, output, c);
    // -- WP MX1 -- Link L/R pairs: the partner's crosspoint becomes the same,
    // both in one write and one undo step
    const QVariantMap mirror = mirrorOf(input, output);
    if (mirror.isEmpty()) {
        cc->setRoute(input, output, c.connected, double(c.gain), c.invert);
        return;
    }
    const int pi = mirror.value(QStringLiteral("input")).toInt(), po = mirror.value(QStringLiteral("output")).toInt();
    m_model->stage(pi, po, c);
    const RouteWrite routes[2] = { { uint8_t(input), uint8_t(output), c.connected, c.gain, c.invert },
                                   { uint8_t(pi), uint8_t(po), c.connected, c.gain, c.invert } };
    const RouteWrite &top = input < pi ? routes[0] : routes[1];
    const QByteArray label = tr("Linked Routing").toUtf8();
    m_ctx.device->write(QStringLiteral("matrix route pair %1 %2").arg(top.input).arg(top.output),
                        [a = routes[0], b = routes[1], label](FfiCore *core) {
                            dspi_undo_group_begin(core, label.constData());
                            bool ok = dspi_set_matrix_route(core, a.input, a.output, a.enabled, a.gain, a.invert);
                            ok = dspi_set_matrix_route(core, b.input, b.output, b.enabled, b.gain, b.invert) && ok;
                            dspi_undo_group_end(core);
                            return ok;
                        });
}

// -- WP MX1 --
void MatrixController::setRoute(int input, int output, bool connected, double gain, bool invert) {
    if (!std::isfinite(gain)) return;
    const float g = float(qBound(GainMin, gain, GainMax));
    editCell(input, output, [connected, g, invert](MatrixModel::Cell &c) {
        c.connected = connected;
        c.gain = g;
        c.invert = invert;
    });
}

void MatrixController::setConnected(int input, int output, bool connected) {
    editCell(input, output, [connected](MatrixModel::Cell &c) { c.connected = connected; });
}

void MatrixController::setGain(int input, int output, double db) {
    if (!std::isfinite(db)) return;
    const float g = float(qBound(GainMin, db, GainMax));
    editCell(input, output, [g](MatrixModel::Cell &c) { c.gain = g; });
}

void MatrixController::setInvert(int input, int output, bool invert) {
    editCell(input, output, [invert](MatrixModel::Cell &c) { c.invert = invert; });
}

void MatrixController::directRouting() {
    const DspState *s = m_ctx.device->state();
    if (!writable() || !s) return;
    const int inputs = qMin(m_ctx.device->channels()->inputCount(), MAX_INPUTS);
    const int outputs = columns();
    const int n = qMin(inputs, outputs);
    const int pdm = pdmOutput();
    const bool pdmOn = pdm >= 0 && pdm < outputs && s->output_enabled[pdm];
    std::vector<RouteWrite> routes;
    for (int i = 0; i < inputs && i < rows(); ++i)
        for (int o = 0; o < outputs; ++o) {
            const bool on = i == o && i < n;
            const MatrixModel::Cell c = m_model->cell(i, o);
            if (c.connected != on || c.gain != 0.0f || c.invert)
                routes.push_back({ uint8_t(i), uint8_t(o), on, 0.0f, false });
        }
    std::vector<uint8_t> enable;
    for (int o = 0; o < n; ++o)
        if (o != pdm && !s->output_enabled[o]) enable.push_back(uint8_t(o));
    m_model->unstageAll();
    // PDM goes off first: that frees Core 1 for the EQ outputs enabled after
    // -- WP P1QT2 -- one undo step (P1UG): the core's group around the writes
    const QByteArray label = tr("Direct 1:1").toUtf8();
    m_ctx.device->write(QStringLiteral("matrix routes"), [pdmOn, pdm, routes, enable, label](FfiCore *c) {
        bool ok = true;
        if (pdmOn) ok = dspi_set_output_enable(c, uint8_t(pdm), false) == 0 && ok;
        // The crosspoints are the step; an output switched on is never undone
        // (audible), so it stays outside the group
        dspi_undo_group_begin(c, label.constData());
        for (const RouteWrite &r : routes)
            ok = dspi_set_matrix_route(c, r.input, r.output, r.enabled, r.gain, r.invert) && ok;
        dspi_undo_group_end(c);
        for (uint8_t o : enable) ok = dspi_set_output_enable(c, o, true) == 1 && ok;
        return ok;
    });
}

void MatrixController::clearRoutes() {
    if (!writable()) return;
    const int inputs = qMin(m_ctx.device->channels()->inputCount(), rows());
    std::vector<RouteWrite> routes;
    for (int i = 0; i < inputs; ++i)
        for (int o = 0; o < columns(); ++o) {
            const MatrixModel::Cell c = m_model->cell(i, o);
            if (c.connected) routes.push_back({ uint8_t(i), uint8_t(o), false, c.gain, c.invert });
        }
    m_model->unstageAll();
    if (routes.empty()) return;
    // -- WP P1QT2 -- one undo step (P1UG)
    const QByteArray label = tr("Clear Routing").toUtf8();
    m_ctx.device->write(QStringLiteral("matrix routes"), [routes, label](FfiCore *c) {
        bool ok = true;
        dspi_undo_group_begin(c, label.constData());
        for (const RouteWrite &r : routes)
            ok = dspi_set_matrix_route(c, r.input, r.output, r.enabled, r.gain, r.invert) && ok;
        dspi_undo_group_end(c);
        return ok;
    });
}

// ---- Outputs -------------------------------------------------------------------

void MatrixController::toggleEnable(int output) {
    const DspState *s = m_ctx.device->state();
    ChannelController *cc = channels();
    if (!writable() || !s || !cc || output < 0 || output >= columns()) return;
    if (s->output_enabled[output]) {
        cc->requestOutputEnable(output, false);
        return;
    }
    if (!cc->enableWouldConflict(output)) {
        cc->requestOutputEnable(output, true);
        return;
    }
    // The window asks; ChannelController's own question is for the output page
    cc->enableAsked();   // -- WP MDQT2a --
    const int pdm = pdmOutput();
    if (output == pdm)
        emit enableQuestion(output, tr("Warning"), tr("Outputs %1-%2 will be disabled. Are you sure?").arg(3).arg(pdm),
                            tr("Enable PDM"));
    else
        emit enableQuestion(output, tr("Warning"), tr("The PDM output will be disabled. Are you sure?"), tr("Disable PDM"));
}

void MatrixController::confirmEnable(int output) {
    ChannelController *cc = channels();
    if (writable() && cc && output >= 0 && output < columns()) cc->confirmOutputEnable(output);
}

void MatrixController::setOutputGain(int output, double db) {
    ChannelController *cc = channels();
    if (!writable() || !cc || !std::isfinite(db) || output < 0 || output >= columns()) return;
    cc->setOutputGain(output, qBound(GainMin, db, OutputGainMax)); // -- WP QPI2 --
}

void MatrixController::setOutputDelay(int output, double ms) {
    ChannelController *cc = channels();
    if (!writable() || !cc || output < 0 || output >= columns()) return;
    // Past the device's line the core refuses it (the delay-refusal banner)
    // and the field shows the delay kept
    cc->setOutputDelay(output, std::isfinite(ms) ? qMax(0.0, ms) : ms);
}

void MatrixController::toggleMute(int output) {
    const DspState *s = m_ctx.device->state();
    ChannelController *cc = channels();
    if (!writable() || !s || !cc || output < 0 || output >= columns()) return;
    cc->setOutputMute(output, !s->output_muted[output]);
}

bool MatrixController::renameOutput(int output, const QString &name) {
    const QString trimmed = name.trimmed();
    const ChannelController *cc = channels();
    if (!writable() || !cc || trimmed.isEmpty() || output < 0 || output >= columns()) return false;
    return m_ctx.device->setChannelName(cc->outputChannel(output), trimmed);
}

// ---- Inputs --------------------------------------------------------------------

void MatrixController::setTrim(int input, double db) {
    ChannelController *cc = channels();
    if (writable() && cc && std::isfinite(db) && input >= 0 && input < m_ctx.device->channels()->inputCount())
        cc->setPreamp(input, db);
}
