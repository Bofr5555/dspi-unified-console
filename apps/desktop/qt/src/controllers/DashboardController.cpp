#include "DashboardController.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "Poller.h"
#include "Snapshot.h"

#include <QJsonObject>
#include <QSettings>

static const Registrar::Controller<DashboardController> dashboardController;

namespace {

const QLatin1String kCardsPerRowKey("dashboard/cardsPerRow");

// printf-style fixed decimals with the zeros at the end of the fraction
// dropped; `keepOne` keeps "3.0" where the fraction was all zeros, otherwise
// the integer part alone is left ("3")
QString trimmed(double value, int decimals, bool sign, bool keepOne) {
    QString text = QString::asprintf(sign ? "%+.*f" : "%.*f", decimals, value);
    const qsizetype dot = text.indexOf(QLatin1Char('.'));
    if (dot < 0) return text;
    qsizetype end = text.size();
    while (end > dot + 1 && text.at(end - 1) == QLatin1Char('0')) --end;
    if (end == dot + 1) return keepOne ? text.left(dot) + QLatin1String(".0") : text.left(dot);
    return text.left(end);
}

} // namespace

DashboardController::DashboardController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    m_cardsPerRow = qBound(0, QSettings().value(kCardsPerRowKey, 0).toInt(), kMaxCardsPerRow);

    connect(ctx.device, &DeviceModel::snapshotApplied, this, &DashboardController::fetchCards);

    m_staleTimer.setSingleShot(true);
    connect(&m_staleTimer, &QTimer::timeout, this, [this]() { setStale(m_ctx.poller && m_ctx.poller->running()); });
    if (ctx.poller) {
        connect(ctx.poller, &Poller::metersChanged, this, &DashboardController::onMeters);
        connect(ctx.poller, &Poller::runningChanged, this, &DashboardController::onRunning);
        connect(ctx.poller, &Poller::slowTransportChanged, this, &DashboardController::onRunning);
    }
    onRunning();
    fetchCards();
}

void DashboardController::setCardsPerRow(int n) {
    n = qBound(0, n, kMaxCardsPerRow);
    if (n == m_cardsPerRow) return;
    m_cardsPerRow = n;
    QSettings().setValue(kCardsPerRowKey, n);
    emit cardsPerRowChanged();
}

int DashboardController::columnsFor(int chosen, qreal width, int count) {
    const int wanted = chosen > 0 ? chosen : int((width + kSpacing) / (kAutoMinCardWidth + kSpacing));
    return qMax(1, qMin(wanted, count));
}

bool DashboardController::clipped(int channel) const {
    return channel >= 0 && channel < 32 && (m_clipLatched >> channel) & 1u;
}

QString DashboardController::typeCode(int type) {
    // The Mac's dashboard codes; passes read as cuts (a low pass is a high cut)
    switch (type) {
    case FILTER_FLAT: return QStringLiteral("OFF");
    case FILTER_PEAKING: return QStringLiteral("PK");
    case FILTER_LOWSHELF: return QStringLiteral("LS");
    case FILTER_HIGHSHELF: return QStringLiteral("HS");
    case FILTER_LOWPASS: return QStringLiteral("HC");
    case FILTER_HIGHPASS: return QStringLiteral("LC");
    case FILTER_NOTCH: return QStringLiteral("NO");
    case FILTER_ALLPASS: return QStringLiteral("AP");
    case FILTER_ALLPASS1: return QStringLiteral("AP1");
    case FILTER_LOWSHELF1: return QStringLiteral("LS1");
    case FILTER_HIGHSHELF1: return QStringLiteral("HS1");
    case FILTER_LINKWITZ_TRANSFORM: return QStringLiteral("LT");
    case FILTER_LOWPASS1: return QStringLiteral("HC1");
    case FILTER_HIGHPASS1: return QStringLiteral("LC1");
    default: return QString::number(type);
    }
}

bool DashboardController::usesGain(int type) {
    return type == FILTER_PEAKING || type == FILTER_LOWSHELF || type == FILTER_HIGHSHELF ||
           type == FILTER_LOWSHELF1 || type == FILTER_HIGHSHELF1;
}

bool DashboardController::showsQ(int type) { return type == FILTER_PEAKING; }

QString DashboardController::freqText(double hz) { return QString::asprintf("%.0f", hz); }

QString DashboardController::gainText(double db) { return trimmed(db, 2, true, true); }

QString DashboardController::qText(double q) { return trimmed(q, 3, false, true); }

QString DashboardController::delayText(double ms) { return trimmed(ms, 4, false, false); }

int DashboardController::staleAfterMs() const {
    // Over a slow transport the meters come at 2 Hz: allow four of them
    return m_ctx.poller && m_ctx.poller->slowTransport() ? 4 * Poller::kSlowIntervalMs : kStaleMs;
}

// The output cards come from the core's mirror; one read at a time, and one
// more if the state changed meanwhile
void DashboardController::fetchCards() {
    if (m_cardsInFlight) {
        m_cardsAgain = true;
        return;
    }
    if (!m_ctx.device->connected()) {
        m_outputCards = QJsonArray();
        rebuild();
        return;
    }
    m_cardsInFlight = true;
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            return jsonValue(readSized([c](char *b, uint32_t n) { return dspi_output_cards(c, b, n); })).toArray();
        },
        [this](const QJsonArray &cards) {
            m_cardsInFlight = false;
            m_outputCards = cards;
            rebuild();
            if (m_cardsAgain) {
                m_cardsAgain = false;
                fetchCards();
            }
        });
}

QVariantMap DashboardController::column(int wire, const QString &heading, bool output, int number,
                                        bool showDelay) const {
    const DspState *s = m_ctx.device->state();
    const ChannelListModel *rows = m_ctx.device->channels();
    const int row = rows->rowOfChannel(wire);
    QVariantMap c;
    c.insert(QStringLiteral("channel"), wire);
    c.insert(QStringLiteral("heading"),
             heading.isEmpty() && row >= 0 ? rows->get(row, QStringLiteral("label")).toString() : heading);
    c.insert(QStringLiteral("output"), output);
    c.insert(QStringLiteral("number"), number);
    c.insert(QStringLiteral("pdm"), row >= 0 && rows->get(row, QStringLiteral("pdm")).toBool());
    const bool outputOk = s && output && number >= 0 && number < MAX_OUTPUTS;
    c.insert(QStringLiteral("delay"),
             showDelay && outputOk ? tr("Delay: %1ms").arg(delayText(s->output_delay_ms[number])) : QString());
    c.insert(QStringLiteral("muted"), outputOk && s->output_muted[number]);
    return c;
}

void DashboardController::rebuild() {
    QVariantList cards;
    const DspState *s = m_ctx.device->state();
    if (s && m_ctx.device->connected()) {
        // The stereo input card: the first two inputs, no delay readout
        QVariantMap input;
        input.insert(QStringLiteral("key"), QStringLiteral("input"));
        input.insert(QStringLiteral("columns"), QVariantList{ column(0, QString(), false, 0, false),
                                                              column(1, QString(), false, 1, false) });
        cards.append(input);

        const int inputs = int(s->num_input_channels);
        const int outputs = int(s->num_output_channels);
        for (const QJsonValue &v : std::as_const(m_outputCards)) {
            const QJsonObject o = v.toObject();
            const int left = o.value(QLatin1String("left")).toInt(-1);
            const QJsonValue rightValue = o.value(QLatin1String("right"));
            const int right = rightValue.isDouble() ? rightValue.toInt(-1) : -1;
            if (left < 0 || left >= outputs || right >= outputs) continue;
            QVariantList columns{ column(inputs + left, o.value(QLatin1String("leftName")).toString(), true, left, true) };
            if (right >= 0)
                columns.append(column(inputs + right, o.value(QLatin1String("rightName")).toString(), true, right, true));
            QVariantMap card;
            card.insert(QStringLiteral("key"),
                        right >= 0 ? QStringLiteral("out%1-%2").arg(left).arg(right) : QStringLiteral("out%1").arg(left));
            card.insert(QStringLiteral("columns"), columns);
            cards.append(card);
        }
    }
    if (cards == m_cards) return;
    m_cards = cards;
    emit cardsChanged();
}

void DashboardController::onMeters() {
    // A reading came: the meters are current
    setStale(false);
    m_staleTimer.start(staleAfterMs());

    // The Mac's latch: new clip bits are held, and the hold restarts with each
    // new one; after the hold with none new, the marks and the device's flags clear
    const uint flags = m_ctx.poller->clipFlags();
    const uint fresh = flags & ~m_clipLatched;
    if (fresh) {
        m_clipLatched |= flags;
        m_lastClip.start();
        emit clipLatchedChanged();
    } else if (m_clipLatched && m_lastClip.isValid() && m_lastClip.elapsed() > kClipHoldMs) {
        m_clipLatched = 0;
        m_lastClip.invalidate();
        emit clipLatchedChanged();
        // -- WP QP16 -- A read-only device takes the clear too (behaviour.md §14.2)
        m_ctx.device->writeEvenReadOnly(QStringLiteral("clear clips"), [](FfiCore *c) { return dspi_clear_clips(c); });
    }
}

void DashboardController::onRunning() {
    if (m_ctx.poller && m_ctx.poller->running()) {
        m_staleTimer.start(staleAfterMs());
    } else {
        // No device: nothing is late, and the marks belong to the device that went
        m_staleTimer.stop();
        setStale(false);
        if (m_clipLatched) {
            m_clipLatched = 0;
            m_lastClip.invalidate();
            emit clipLatchedChanged();
        }
    }
}

void DashboardController::setStale(bool stale) {
    if (stale == m_stale) return;
    m_stale = stale;
    emit staleChanged();
}
