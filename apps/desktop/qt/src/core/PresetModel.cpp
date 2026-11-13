#include "PresetModel.h"

#include "Snapshot.h"

int PresetModel::rowCount(const QModelIndex &parent) const { return parent.isValid() ? 0 : int(m_entries.size()); }

QVariant PresetModel::data(const QModelIndex &index, int role) const {
    if (!index.isValid() || index.row() < 0 || index.row() >= m_entries.size()) return QVariant();
    const Slot &s = m_entries.at(index.row());
    switch (role) {
    case SlotRole: return index.row();
    case Qt::DisplayRole:
    case NameRole: return s.name;
    case OccupiedRole: return s.occupied;
    case ActiveRole: return index.row() == m_active;
    default: return QVariant();
    }
}

QHash<int, QByteArray> PresetModel::roleNames() const {
    return { { SlotRole, "slot" }, { NameRole, "name" }, { OccupiedRole, "occupied" }, { ActiveRole, "active" } };
}

void PresetModel::update(const DeviceSnapshot &snap) {
    QVector<Slot> entries;
    int active = -1, startup = 0, def = 0;
    if (const DspState *s = snap.state.get()) {
        for (int i = 0; i < MAX_PRESETS; ++i)
            entries.append({ fieldText(s->preset_names[i], CHANNEL_NAME_LEN), (s->preset_occupied >> i & 1) != 0 });
        active = s->active_preset_slot < MAX_PRESETS ? s->active_preset_slot : -1;
        startup = s->preset_startup_mode;
        def = s->preset_default_slot;
    }
    const int oldActive = m_active;
    m_active = active;
    if (entries.size() != m_entries.size()) {
        beginResetModel();
        m_entries = entries;
        endResetModel();
        emit countChanged();
    } else {
        for (int i = 0; i < entries.size(); ++i) {
            if (entries.at(i) == m_entries.at(i) && i != oldActive && i != active) continue;
            m_entries[i] = entries.at(i);
            emit dataChanged(index(i), index(i));
        }
    }
    if (oldActive != active || startup != m_startupMode || def != m_defaultSlot) {
        m_startupMode = startup;
        m_defaultSlot = def;
        emit slotsChanged();
    }
}
