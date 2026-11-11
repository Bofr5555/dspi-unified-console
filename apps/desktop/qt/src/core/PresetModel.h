#ifndef PRESETMODEL_H
#define PRESETMODEL_H

#include <QAbstractListModel>
#include <QVector>
#include <QtQml/qqmlregistration.h>

struct DeviceSnapshot;

// The device's preset slots from the last snapshot: name, occupied, active.
// QML reads it as DeviceModel.presets. Loading, saving and the unsaved-changes
// guard belong to the preset controller.
class PresetModel : public QAbstractListModel {
    Q_OBJECT
    QML_ELEMENT
    QML_UNCREATABLE("DeviceModel.presets")
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)
    Q_PROPERTY(int activeSlot READ activeSlot NOTIFY slotsChanged)
    Q_PROPERTY(int startupMode READ startupMode NOTIFY slotsChanged)
    Q_PROPERTY(int defaultSlot READ defaultSlot NOTIFY slotsChanged)
public:
    enum Role {
        SlotRole = Qt::UserRole + 1, // from 0
        NameRole,                    // "" for an unnamed slot
        OccupiedRole,
        ActiveRole,
    };
    Q_ENUM(Role)

    explicit PresetModel(QObject *parent = nullptr) : QAbstractListModel(parent) {}

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    int activeSlot() const { return m_active; }
    int startupMode() const { return m_startupMode; }
    int defaultSlot() const { return m_defaultSlot; }

    void update(const DeviceSnapshot &snap);

signals:
    void countChanged();
    void slotsChanged();

private:
    struct Slot {
        QString name;
        bool occupied = false;
        bool operator==(const Slot &o) const { return name == o.name && occupied == o.occupied; }
    };
    QVector<Slot> m_entries;
    int m_active = -1;
    int m_startupMode = 0;
    int m_defaultSlot = 0;
};

#endif // PRESETMODEL_H
