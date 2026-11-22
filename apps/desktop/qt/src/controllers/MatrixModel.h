#ifndef MATRIXMODEL_H
#define MATRIXMODEL_H

#include <QAbstractListModel>
#include <QHash>
#include <QVector>
#include <QtQml/qqmlregistration.h>
#include <utility>

struct DspState;

// -- WP QP6 -- The matrix mixer's crosspoints, one item per (source row,
// output), row by row: what MatrixWindow's grid shows. Filled from the
// snapshot by MatrixController; a crosspoint edit is staged here (the cell
// shows it at once) until a snapshot carries it or the device goes idle, so a
// connect followed quickly by INV never sends the INV with the old connect.
class MatrixModel : public QAbstractListModel {
    Q_OBJECT
    QML_ELEMENT
    QML_UNCREATABLE("MatrixController.model")
    Q_PROPERTY(int rows READ rows NOTIFY shapeChanged)
    Q_PROPERTY(int columns READ columns NOTIFY shapeChanged)
public:
    enum Role {
        InputRole = Qt::UserRole + 1, // the source row (an input, or a derived upmix row)
        OutputRole,                   // the output, from 0
        ConnectedRole,
        GainRole,                     // dB
        InvertRole,
        OutputEnabledRole,            // the column's output is on (off: the cell is dimmed)
        ConflictRole,                 // the PDM column while enabling PDM would collide on Core 1
    };
    Q_ENUM(Role)

    struct Cell {
        bool connected = false;
        float gain = 0;
        bool invert = false;
        bool operator==(const Cell &o) const { return connected == o.connected && gain == o.gain && invert == o.invert; }
    };

    explicit MatrixModel(QObject *parent = nullptr) : QAbstractListModel(parent) {}

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    int rows() const { return m_rows; }
    int columns() const { return m_cols; }

    // The crosspoint as shown: a staged edit wins over the snapshot
    Cell cell(int input, int output) const;
    bool valid(int input, int output) const { return input >= 0 && input < m_rows && output >= 0 && output < m_cols; }
    // A role of one crosspoint by name, for QML outside a delegate
    Q_INVOKABLE QVariant get(int input, int output, const QString &role) const;

    // From a snapshot: `rows` source rows by `cols` outputs. Staged edits the
    // snapshot now carries are dropped.
    void update(const DspState *s, int rows, int cols, const QVector<bool> &conflict);
    void stage(int input, int output, const Cell &c);
    // Drops one staged edit (a refused write), or all of them
    void unstage(int input, int output);
    void unstageAll();
    // -- WP MDQT2a -- Another device's staged edits in place of these, which are
    // returned; update() follows and shows them
    QHash<int, Cell> swapStaged(const QHash<int, Cell> &staged) { return std::exchange(m_staged, staged); }
    int stagedCount() const { return int(m_staged.size()); }

signals:
    void shapeChanged();

private:
    struct Shown {
        Cell cell;
        bool outputEnabled = false;
        bool conflict = false;
        bool operator==(const Shown &o) const {
            return cell == o.cell && outputEnabled == o.outputEnabled && conflict == o.conflict;
        }
    };
    int key(int input, int output) const { return input * 16 + output; }
    void refreshCell(int input, int output);

    int m_rows = 0;
    int m_cols = 0;
    QVector<Cell> m_device;   // from the snapshot, row by row
    QVector<Shown> m_shown;   // what the view has
    QVector<bool> m_enabled;  // per output
    QVector<bool> m_conflict; // per output
    QHash<int, Cell> m_staged;
};

#endif // MATRIXMODEL_H
