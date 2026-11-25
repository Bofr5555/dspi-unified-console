#ifndef CHANNELLISTMODEL_H
#define CHANNELLISTMODEL_H

#include <QAbstractListModel>
#include <QVector>
#include <QtQml/qqmlregistration.h>

struct DeviceSnapshot;
struct DspState;

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
        // -- WP SHOWFIX -- the names a person knows the channel by (dspi_channel_words)
        DisplayNameRole,             // what a list of channels shows: the speaker layout's
                                     // "Centre woofer", else the label (the Overview's name)
        ShortNameRole,               // what a sentence names it by: "Centre woofer", "LFE", "Out 3"
        // -- WP INTE -- (owner 2026-11-24) outputs: nothing is routed to it
        // (dspi_output_labels' `unused`); a view that only shows leaves it out
        UnusedRole,
    };
    Q_ENUM(Role)

    explicit ChannelListModel(QObject *parent = nullptr) : QAbstractListModel(parent) {}

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;
    // The input rows: the strips the input source feeds (effectiveInputs).
    // The wire's input count (where the outputs start) is DspState's.
    int inputCount() const;
    static int effectiveInputs(const DspState &s); // -- WP QPI2 --
    // -- WP QP18 -- The OS's channel count to the open DSPi (HostAudioFormat),
    // 0 when it gives none: on USB it decides the input strips
    static void setHostInputs(int channels);

    // The row of a wire channel, -1 if none.
    Q_INVOKABLE int rowOfChannel(int channel) const;
    // A row's role by name, for QML code outside a delegate ("label", "channel", ...)
    Q_INVOKABLE QVariant get(int row, const QString &role) const;
    // -- WP FIN015 -- (REVF D1) one channel, one name on a page. An output's
    // name where it is not "Out N" ("Centre woofer"), else "" (output from 0)
    Q_INVOKABLE QString outputName(int output) const;
    // The core's words as a person reads them beside the pages: each "Out N"
    // followed by the output's name, "Out 1 (Centre woofer)". For the shown
    // text only (Events, the AI question sheet); an agent gets the core's.
    Q_INVOKABLE QString withNames(const QString &text) const;

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
        QString displayName; // -- WP SHOWFIX --
        QString shortName;   // -- WP SHOWFIX --
        bool unused = false; // -- WP INTE --
        bool operator==(const Row &o) const {
            return output == o.output && index == o.index && channel == o.channel && name == o.name &&
                   label == o.label && enabled == o.enabled && userNamed == o.userNamed && pdm == o.pdm &&
                   displayName == o.displayName && shortName == o.shortName && unused == o.unused;
        }
    };
    QVector<Row> m_rows;
};

#endif // CHANNELLISTMODEL_H
