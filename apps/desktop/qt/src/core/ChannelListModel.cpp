#include "ChannelListModel.h"

#include "Snapshot.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QRegularExpression>

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
    case DisplayNameRole: return r.displayName; // -- WP SHOWFIX --
    case ShortNameRole: return r.shortName;
    case UnusedRole: return r.unused; // -- WP INTE --
    case HiddenRole: return r.hidden; // -- WP FIN015 --
    default: return QVariant();
    }
}

QHash<int, QByteArray> ChannelListModel::roleNames() const {
    return {
        { KindRole, "kind" },         { IndexRole, "number" },     { ChannelRole, "channel" },
        { NameRole, "name" },         { LabelRole, "label" },     { DescriptorRole, "descriptor" },
        { EnabledRole, "outputEnabled" },   { UserNamedRole, "userNamed" }, { PdmRole, "pdm" },
        { DisplayNameRole, "displayName" }, { ShortNameRole, "shortName" }, // -- WP SHOWFIX --
        { UnusedRole, "unused" },                                           // -- WP INTE --
        { HiddenRole, "hidden" },                                           // -- WP FIN015 --
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

// -- WP FIN015 -- (REVF D1)
QString ChannelListModel::outputName(int output) const {
    for (const Row &r : m_rows) {
        if (!r.output || r.index != output) continue;
        // "Out 1", "Out 1 · R · Low": the number's own word says it already
        static const QRegularExpression own(QStringLiteral("^Out (\\d+)(?!\\d)"));
        const QRegularExpressionMatch m = own.match(r.displayName);
        if (r.displayName.isEmpty() || (m.hasMatch() && m.captured(1).toInt() == output + 1)) return {};
        return r.displayName;
    }
    return {};
}

QString ChannelListModel::withNames(const QString &text) const {
    // "Out N" as the core's words say it, not one already followed by its name
    static const QRegularExpression word(QStringLiteral("\\bOut (\\d+)(?!\\d)(?! \\()"));
    QString out;
    qsizetype at = 0;
    QRegularExpressionMatchIterator it = word.globalMatch(text);
    while (it.hasNext()) {
        const QRegularExpressionMatch m = it.next();
        out += text.mid(at, m.capturedEnd() - at);
        const QString name = outputName(m.captured(1).toInt() - 1);
        if (!name.isEmpty()) out += QStringLiteral(" (%1)").arg(name);
        at = m.capturedEnd();
    }
    return out + text.mid(at);
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
        const QJsonObject &words = snap.channelWords;                                  // -- WP SHOWFIX --
        const QJsonArray fromLayout = words.value(QLatin1String("fromLayout")).toArray(); // -- WP SHOWFIX --
        const int inputs = effectiveInputs(*s);
        for (int i = 0; i < inputs && i < MAX_CHANNELS; ++i) {
            Row r;
            r.index = i;
            r.channel = i;
            r.name = fieldText(s->channel_names[i], CHANNEL_NAME_LEN);
            r.label = r.name.isEmpty() ? tr("USB %1").arg(i + 1) : r.name;
            // -- WP SHOWFIX -- the core's word for it ("LFE", "In 7")
            r.displayName = r.label;
            r.shortName = words.value(QLatin1String("inputs")).toArray().at(i).toString(r.label);
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
            // -- WP INTE -- (owner 2026-11-24) nothing routed to it: the dock's
            // meters leave it out, the sidebar (an edit view) keeps it
            r.unused = o.value(QLatin1String("unused")).toBool();
            // -- WP FIN015 -- (REVF R1) unless the generator plays on it: the
            // generator writes after the matrix, so it is loud
            r.hidden = o.value(QLatin1String("hidden")).toBool();
            // -- WP SHOWFIX -- the speaker layout's name where the core has one
            // ("Centre woofer"), as the Overview's card names it
            const QString word = words.value(QLatin1String("outputs")).toArray().at(r.index).toString();
            r.displayName = fromLayout.contains(QJsonValue(r.index)) && !word.isEmpty() ? word : r.label;
            r.shortName = word.isEmpty() ? r.label : word;
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
