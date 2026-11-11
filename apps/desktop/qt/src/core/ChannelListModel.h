#ifndef CHANNELLISTMODEL_H
#define CHANNELLISTMODEL_H

#include <QAbstractListModel>
#include <QVector>
#include <QtQml/qqmlregistration.h>

struct DeviceSnapshot;

// The sidebar's rows: the device's inputs, then every output, from the last
// snapshot. Output rows carry the core's label ("Out 1 · R · Low" while the
// slot has its default name, the user's name otherwise; dspi_output_labels).
// Filled by DeviceModel; QML reads it as DeviceModel.channels.
class ChannelListModel : public QAbstractListModel {
    Q_OBJECT
    QML_ELEMENT
    QML_UNCREATABLE("DeviceModel.channels")
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)
    Q_PROPERTY(int inputCount READ inputCount NOTIFY countChanged)
public:
    enum Role {
        KindRole = Qt::UserRole + 1, // "input" or "output"
        IndexRole,                   // input or output number, from 0
        ChannelRole,                 // the wire channel (EQ channel)
        NameRole,                    // the channel's name as the device stores it
        LabelRole,                   // what the sidebar shows
        DescriptorRole,              // "IN1", "OUT3"
        EnabledRole,                 // outputs: enabled in the matrix; inputs: true
        UserNamedRole,               // the user named it
        PdmRole,                     // the PDM subwoofer output
    };
    Q_ENUM(Role)

    explicit ChannelListModel(QObject *parent = nullptr) : QAbstractListModel(parent) {}

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;
    int inputCount() const;

    // The row of a wire channel, -1 if none.
    Q_INVOKABLE int rowOfChannel(int channel) const;
    // A row's role by name, for QML code outside a delegate ("label", "channel", ...)
    Q_INVOKABLE QVariant get(int row, const QString &role) const;

    void update(const DeviceSnapshot &snap, const QString &platform);

signals:
    void countChanged();

private:
    struct Row {
        bool output = false;
        int index = 0;
        int channel = 0;
        QString name;
        QString label;
        bool enabled = true;
        bool userNamed = false;
        bool pdm = false;
        bool operator==(const Row &o) const {
            return output == o.output && index == o.index && channel == o.channel && name == o.name &&
                   label == o.label && enabled == o.enabled && userNamed == o.userNamed && pdm == o.pdm;
        }
    };
    QVector<Row> m_rows;
};

#endif // CHANNELLISTMODEL_H
