#include "BandListModel.h"

#include "DeviceModel.h"

namespace {
// The rows' roles, by the names roleNames() gives them.
enum : int { BandRole = Qt::UserRole + 1, TypeRole, FreqRole, QRole, GainRole, BypassRole };
} // namespace

BandListModel::BandListModel(QObject *parent) : QAbstractListModel{ parent }, m_device{ DeviceModel::instance() } {
    if (m_device) connect(m_device, &DeviceModel::snapshotApplied, this, &BandListModel::reload);
}

int BandListModel::rowCount(const QModelIndex &parent) const { return parent.isValid() ? 0 : int(m_bands.size()); }

QVariant BandListModel::data(const QModelIndex &index, int role) const {
    if (!index.isValid() || index.row() < 0 || index.row() >= m_bands.size()) return QVariant();
    const Band &b = m_bands.at(index.row());
    switch (role) {
    case BandRole: return index.row();
    case TypeRole: return b.type;
    case FreqRole: return double(b.freq);
    case QRole: return double(b.q);
    case GainRole: return double(b.gain);
    case BypassRole: return b.bypass;
    default: return QVariant();
    }
}

QHash<int, QByteArray> BandListModel::roleNames() const {
    return { { BandRole, "band" }, { TypeRole, "type" },  { FreqRole, "freq" },
             { QRole, "q" },       { GainRole, "gain" },  { BypassRole, "bypass" } };
}

void BandListModel::setChannelIndex(int index) {
    if (index != m_channel) {
        m_channel = index;
        emit channelChanged();
        reload();
    }
}

void BandListModel::reload() {
    QVector<Band> bands;
    const DspState *s = m_device ? m_device->state() : nullptr;
    if (s && m_channel >= 0 && m_channel < MAX_CHANNELS) {
        for (int i = 0; i < BANDS_PER_CHANNEL; ++i) {
            const FilterParams &f = s->filters[m_channel][i];
            bands.append({ f.freq, f.q, f.gain, int(f.filter_type), f.bypass });
        }
    }
    if (bands.size() != m_bands.size()) {
        beginResetModel();
        m_bands = bands;
        endResetModel();
        emit countChanged();
        return;
    }
    for (int i = 0, n = int(bands.size()); i < n; ++i) {
        const Band &a = bands.at(i), &b = m_bands.at(i);
        if (a.freq == b.freq && a.q == b.q && a.gain == b.gain && a.type == b.type && a.bypass == b.bypass) continue;
        m_bands[i] = bands.at(i);
        emit dataChanged(index(i), index(i));
    }
}
