#ifndef BANDLISTMODEL_H
#define BANDLISTMODEL_H

#include <QtQml/qqmlregistration.h>
#include <QAbstractListModel>
#include <QList>
#include <QPointer>

class DeviceModel;

// One channel's PEQ bands from DeviceModel's state, for a filter list:
//
//     BandListModel { channel: 3 }
//
// Each row offers band (from 0), type (the firmware's filter type code),
// freq (Hz), q, gain (dB) and bypass. Read-only; edits go through the filter
// controller's write path.
class BandListModel : public QAbstractListModel {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(int channel READ channelIndex WRITE setChannelIndex NOTIFY channelChanged FINAL)
    Q_PROPERTY(int count READ bandCount NOTIFY countChanged FINAL)

public:
    explicit BandListModel(QObject *parent = nullptr);

    QHash<int, QByteArray> roleNames() const override;
    QVariant data(const QModelIndex &index, int role) const override;
    int rowCount(const QModelIndex &parent = {}) const override;

    int bandCount() const { return int(m_bands.size()); }
    int channelIndex() const { return m_channel; }
    void setChannelIndex(int index);

Q_SIGNALS:
    void countChanged();
    void channelChanged();

private:
    // A band as the list shows it; compared whole to find the rows that moved.
    struct Band {
        float freq, q, gain;
        int type;
        bool bypass;
    };
    void reload();

    int m_channel = -1;
    QList<Band> m_bands;
    QPointer<DeviceModel> m_device;
};

#endif // BANDLISTMODEL_H
