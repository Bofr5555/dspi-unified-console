#ifndef BANDLISTMODEL_H
#define BANDLISTMODEL_H

#include <QAbstractListModel>
#include <QPointer>
#include <QVector>
#include <QtQml/qqmlregistration.h>

class DeviceModel;

// One channel's PEQ bands from DeviceModel's state, for a filter list:
//
//     BandListModel { channel: 3 }
//
// Read-only; edits go through the filter controller's write path.
class BandListModel : public QAbstractListModel {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(int channel READ channel WRITE setChannel NOTIFY channelChanged)
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)
public:
    enum Role {
        BandRole = Qt::UserRole + 1, // band number, from 0
        TypeRole,                    // the firmware's filter type code
        FreqRole,                    // Hz
        QRole,
        GainRole,                    // dB
        BypassRole,
    };
    Q_ENUM(Role)

    explicit BandListModel(QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    int channel() const { return m_channel; }
    void setChannel(int channel);

signals:
    void channelChanged();
    void countChanged();

private:
    struct Band {
        int type = 0;
        float freq = 0, q = 0, gain = 0;
        bool bypass = false;
        bool operator==(const Band &o) const {
            return type == o.type && freq == o.freq && q == o.q && gain == o.gain && bypass == o.bypass;
        }
    };
    void reload();

    QPointer<DeviceModel> m_device;
    int m_channel = -1;
    QVector<Band> m_bands;
};

#endif // BANDLISTMODEL_H
