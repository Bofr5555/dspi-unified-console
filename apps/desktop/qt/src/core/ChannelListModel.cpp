#include "ChannelListModel.h"

#include "Snapshot.h"

#include <QJsonObject>

int ChannelListModel::rowCount(const QModelIndex &parent) const { return parent.isValid() ? 0 : int(m_rows.size()); }

int ChannelListModel::inputCount() const {
    int n = 0;
    for (const Row &r : m_rows) n += r.output ? 0 : 1;
    return n;
}

QVariant ChannelListModel::data(const QModelIndex &index, int role) const {
    if (!index.isValid() || index.row() < 0 || index.row() >= m_rows.size()) return QVariant();
    const Row &r = m_rows.at(index.row());
    switch (role) {
    case KindRole: return r.output ? QStringLiteral("output") : QStringLiteral("input");
    case IndexRole: return r.index;
    case ChannelRole: return r.channel;
    case NameRole: return r.name;
    case Qt::DisplayRole:
    case LabelRole: return r.label;
    case DescriptorRole:
        return r.output ? QStringLiteral("OUT%1").arg(r.index + 1) : QStringLiteral("IN%1").arg(r.index + 1);
    case EnabledRole: return r.enabled;
    case UserNamedRole: return r.userNamed;
    case PdmRole: return r.pdm;
    default: return QVariant();
    }
}

QHash<int, QByteArray> ChannelListModel::roleNames() const {
    return {
        { KindRole, "kind" },         { IndexRole, "number" },     { ChannelRole, "channel" },
        { NameRole, "name" },         { LabelRole, "label" },     { DescriptorRole, "descriptor" },
        { EnabledRole, "outputEnabled" },   { UserNamedRole, "userNamed" }, { PdmRole, "pdm" },
    };
}

int ChannelListModel::rowOfChannel(int channel) const {
    for (int i = 0; i < m_rows.size(); ++i)
        if (m_rows.at(i).channel == channel) return i;
    return -1;
}

QVariant ChannelListModel::get(int row, const QString &role) const {
    const QByteArray name = role.toUtf8();
    const QHash<int, QByteArray> roles = roleNames();
    for (auto it = roles.cbegin(); it != roles.cend(); ++it)
        if (it.value() == name) return data(index(row), it.key());
    return QVariant();
}

// -- WP QPI2 -- The input strips the source feeds (the Mac's
// effectiveInputChannels): S/PDIF 2, I2S its channel count, ADAT 8; USB the
// OS's format (QP18, HostAudioFormat), or the device's count when the OS
// gives none. At least the stereo pair, at most the device's inputs.
static int s_hostInputs = 0;   // -- WP QP18 --
void ChannelListModel::setHostInputs(int channels) { s_hostInputs = channels; }

int ChannelListModel::effectiveInputs(const DspState &s) {
    const int device = qBound(2, int(s.num_input_channels), MAX_INPUTS);
    int fed = device;
    switch (s.input_source) {
    case 1: case 4: case 5: case 6: fed = 2; break;          // S/PDIF 1-4
    case 2: fed = int(s.i2s_input_channels); break;           // I2S
    case 3: fed = 8; break;                                   // ADAT
    default: if (s_hostInputs > 0) fed = s_hostInputs; break; // USB
    }
    return qBound(2, fed, device);
}

void ChannelListModel::update(const DeviceSnapshot &snap, const QString &platform) {
    QVector<Row> rows;
    if (const DspState *s = snap.state.get()) {
        const int inputs = effectiveInputs(*s);
        for (int i = 0; i < inputs && i < MAX_CHANNELS; ++i) {
            Row r;
            r.index = i;
            r.channel = i;
            r.name = fieldText(s->channel_names[i], CHANNEL_NAME_LEN);
            r.label = r.name.isEmpty() ? tr("USB %1").arg(i + 1) : r.name;
            rows.append(r);
        }
        // The PDM subwoofer's output number, as the Mac app places it
        const int pdmOutput = platform == QLatin1String("RP2350") ? 8 : platform == QLatin1String("RP2040") ? 4 : -1;
        for (const QJsonValue &v : snap.outputLabels) {
            const QJsonObject o = v.toObject();
            Row r;
            r.output = true;
            r.index = o.value(QLatin1String("output")).toInt();
            // Outputs follow the device's inputs in wire order, as the core labels them
            r.channel = int(s->num_input_channels) + r.index;
            r.label = o.value(QLatin1String("label")).toString();
            r.userNamed = o.value(QLatin1String("userNamed")).toBool();
            r.name = r.channel >= 0 && r.channel < MAX_CHANNELS ? fieldText(s->channel_names[r.channel], CHANNEL_NAME_LEN)
                                                                : QString();
            r.enabled = r.index >= 0 && r.index < MAX_OUTPUTS ? s->output_enabled[r.index] : false;
            r.pdm = r.index == pdmOutput;
            rows.append(r);
        }
    }
    if (rows.size() != m_rows.size()) {
        beginResetModel();
        m_rows = rows;
        endResetModel();
        emit countChanged();
        return;
    }
    // Same rows: tell the views which changed, so a rename does not rebuild the sidebar
    for (int i = 0; i < rows.size(); ++i) {
        if (rows.at(i) == m_rows.at(i)) continue;
        m_rows[i] = rows.at(i);
        emit dataChanged(index(i), index(i));
    }
}
