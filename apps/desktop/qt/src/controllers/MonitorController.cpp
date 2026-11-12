// -- WP QP9 -- The Interrupt Monitor's controller (MonitorController.h).
#include "MonitorController.h"

#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"

#include <QClipboard>
#include <QDateTime>
#include <QGuiApplication>
#include <vector>

static const Registrar::Controller<MonitorController> monitorController;
static const Registrar::Window monitorWindow("tools.interruptMonitor", "MonitorWindow");

int MonitorLines::rowCount(const QModelIndex &parent) const {
    return parent.isValid() ? 0 : int(m_lines.size());
}

QVariant MonitorLines::data(const QModelIndex &index, int role) const {
    if (role != LineRole || !index.isValid() || index.row() >= m_lines.size()) return {};
    return m_lines.at(index.row());
}

QHash<int, QByteArray> MonitorLines::roleNames() const {
    return { { LineRole, "line" } };
}

void MonitorLines::append(const QStringList &lines) {
    if (lines.isEmpty()) return;
    // Only the newest kLimit of this batch can stay
    const QStringList tail = lines.size() > kLimit ? lines.mid(lines.size() - kLimit) : lines;
    const int drop = int(m_lines.size() + tail.size()) - kLimit;
    if (drop > 0) {
        beginRemoveRows(QModelIndex(), 0, drop - 1);
        m_lines.remove(0, drop);
        endRemoveRows();
    }
    const int first = int(m_lines.size());
    beginInsertRows(QModelIndex(), first, first + int(tail.size()) - 1);
    m_lines.append(tail);
    endInsertRows();
    emit countChanged();
}

void MonitorLines::clear() {
    if (m_lines.isEmpty()) return;
    beginResetModel();
    m_lines.clear();
    endResetModel();
    emit countChanged();
}

MonitorController::MonitorController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx), m_lines(new MonitorLines(this)) {
    if (ctx.poller) {
        m_poll = ctx.poller->add(
            QStringLiteral("interrupt monitor"), kPollMs, this,
            [last = m_last](FfiCore *c) { return readAfter(c, last->load()); },
            [this](const Batch &b) { apply(b); });
        updatePoll();
    }
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &MonitorController::stateTextChanged);
}

MonitorController::Batch MonitorController::readAfter(FfiCore *core, uint64_t after) {
    Batch batch;
    batch.last = after;
    std::vector<char> buf(kReadBytes);
    // The core stops at a full buffer; a few reads take everything it keeps
    for (int round = 0; round < 8; ++round) {
        const uint64_t next = dspi_monitor_read(core, batch.last, buf.data(), uint32_t(buf.size()));
        if (next == batch.last) break;
        batch.last = next;
        const QString text = QString::fromUtf8(buf.data());
        for (const QString &line : text.split(QLatin1Char('\n'), Qt::SkipEmptyParts)) batch.lines.append(line);
    }
    return batch;
}

void MonitorController::apply(const Batch &batch) {
    if (batch.last <= m_last->load()) return;
    m_last->store(batch.last);
    if (m_paused || m_skipping || batch.lines.isEmpty()) return;
    QStringList rows;
    rows.reserve(batch.lines.size());
    for (const QString &line : batch.lines) rows.append(shown(line));
    m_lines->append(rows);
    emit appended();
}

QString MonitorController::shown(const QString &coreLine) {
    const qsizetype tab = coreLine.indexOf(QLatin1Char('\t'));
    if (tab <= 0) return coreLine;
    bool ok = false;
    const qint64 ms = coreLine.left(tab).toLongLong(&ok);
    if (!ok) return coreLine;
    const QString time = QDateTime::fromMSecsSinceEpoch(ms).toLocalTime().toString(QStringLiteral("HH:mm:ss.zzz"));
    return time + QStringLiteral("  ") + coreLine.mid(tab + 1);
}

void MonitorController::setPaused(bool paused) {
    if (m_paused == paused) return;
    m_paused = paused;
    if (!paused) skipToEnd();
    updatePoll();
    emit pausedChanged();
    emit stateTextChanged();
}

void MonitorController::skipToEnd() {
    // What was logged while paused is dropped, as the Mac's log drops it. The
    // worker runs jobs in order, so a poll read before this is applied (and
    // dropped) before it, and one read after it starts past the skipped lines.
    m_skipping = true;
    m_ctx.core->worker()->call(
        this,
        // One read takes all the core keeps (kLimit lines fit in its rounds)
        [last = m_last](FfiCore *c) { return readAfter(c, last->load()).last; },
        [this](uint64_t end) {
            if (end > m_last->load()) m_last->store(end);
            m_skipping = false;
        });
}

void MonitorController::setWindowShown(bool shown) {
    if (m_shown == shown) return;
    m_shown = shown;
    updatePoll();
    emit windowShownChanged();
}

bool MonitorController::paused() const { return m_paused; }
bool MonitorController::windowShown() const { return m_shown; }

QString MonitorController::status() const {
    if (!m_ctx.device->connected()) return QStringLiteral("inactive");
    return m_paused ? QStringLiteral("paused") : QStringLiteral("listening");
}

void MonitorController::updatePoll() {
    // Paused, the core keeps logging; Resume skips it
    if (m_ctx.poller) m_ctx.poller->setEnabled(m_poll, m_shown && !m_paused);
}

void MonitorController::clear() {
    m_lines->clear();
    m_ctx.core->worker()->post([](FfiCore *c) { dspi_monitor_clear(c); });
}

QString MonitorController::text() const {
    return m_lines->lines().isEmpty() ? QString() : m_lines->lines().join(QLatin1Char('\n')) + QLatin1Char('\n');
}

void MonitorController::copyAll() const {
    if (QClipboard *clip = QGuiApplication::clipboard()) clip->setText(text());
}
