// -- WP QP6 -- The matrix mixer's crosspoints (MatrixModel.h).
#include "MatrixModel.h"

#include "CoreApi.h"

#include <cmath>

int MatrixModel::rowCount(const QModelIndex &parent) const { return parent.isValid() ? 0 : int(m_shown.size()); }

QHash<int, QByteArray> MatrixModel::roleNames() const {
    return { { InputRole, "input" },
             { OutputRole, "output" },
             { ConnectedRole, "connected" },
             { GainRole, "gain" },
             { InvertRole, "invert" },
             { OutputEnabledRole, "outputEnabled" },
             { ConflictRole, "conflict" } };
}

QVariant MatrixModel::data(const QModelIndex &index, int role) const {
    if (!index.isValid() || index.row() >= m_shown.size() || m_cols <= 0) return {};
    const Shown &s = m_shown.at(index.row());
    switch (role) {
    case InputRole: return index.row() / m_cols;
    case OutputRole: return index.row() % m_cols;
    case ConnectedRole: return s.cell.connected;
    case GainRole: return double(s.cell.gain);
    case InvertRole: return s.cell.invert;
    case OutputEnabledRole: return s.outputEnabled;
    case ConflictRole: return s.conflict;
    default: return {};
    }
}

QVariant MatrixModel::get(int input, int output, const QString &role) const {
    if (!valid(input, output)) return {};
    const QByteArray name = role.toLatin1();
    const QHash<int, QByteArray> roles = roleNames();
    for (auto it = roles.cbegin(); it != roles.cend(); ++it)
        if (it.value() == name) return data(index(input * m_cols + output), it.key());
    return {};
}

// -- WP MX1 --
QVariantList MatrixModel::feeds(int input) const {
    QVariantList out;
    for (int o = 0; o < m_cols; ++o)
        if (valid(input, o) && m_shown.at(input * m_cols + o).cell.connected) out << o;
    return out;
}

QVariantList MatrixModel::sources(int output) const {
    QVariantList in;
    for (int i = 0; i < m_rows; ++i)
        if (valid(i, output) && m_shown.at(i * m_cols + output).cell.connected) in << i;
    return in;
}

int MatrixModel::connectedCount() const {
    int n = 0;
    for (const Shown &s : m_shown) n += s.cell.connected ? 1 : 0;
    return n;
}

MatrixModel::Cell MatrixModel::cell(int input, int output) const {
    if (!valid(input, output)) return {};
    const auto staged = m_staged.constFind(key(input, output));
    return staged != m_staged.cend() ? *staged : m_device.at(input * m_cols + output);
}

void MatrixModel::update(const DspState *s, int rows, int cols, const QVector<bool> &conflict) {
    rows = s ? qBound(0, rows, MAX_INPUTS) : 0;
    cols = s ? qBound(0, cols, MAX_OUTPUTS) : 0;
    QVector<Cell> device(rows * cols);
    m_enabled = QVector<bool>(cols);
    m_conflict = QVector<bool>(cols);
    for (int o = 0; o < cols; ++o) {
        m_enabled[o] = s->output_enabled[o];
        m_conflict[o] = conflict.value(o);
    }
    for (int i = 0; i < rows; ++i)
        for (int o = 0; o < cols; ++o)
            device[i * cols + o] = { s->matrix_routing[i][o], s->matrix_gain[i][o], s->matrix_invert[i][o] };

    // A staged edit the device now shows has arrived
    for (auto it = m_staged.begin(); it != m_staged.end();) {
        const int i = it.key() / 16, o = it.key() % 16;
        const bool inside = i < rows && o < cols;
        const Cell &d = inside ? device.at(i * cols + o) : Cell{};
        const bool arrived = inside && d.connected == it->connected && d.invert == it->invert &&
                             std::abs(d.gain - it->gain) < 0.005f;
        it = (!inside || arrived) ? m_staged.erase(it) : std::next(it);
    }

    if (rows != m_rows || cols != m_cols) {
        beginResetModel();
        m_rows = rows;
        m_cols = cols;
        m_device = device;
        m_shown = QVector<Shown>(rows * cols);
        for (int i = 0; i < rows; ++i)
            for (int o = 0; o < cols; ++o) m_shown[i * cols + o] = { cell(i, o), m_enabled.at(o), m_conflict.at(o) };
        endResetModel();
        emit shapeChanged();
        ++m_version;   // -- WP MX1 --
        emit cellsChanged();
        return;
    }
    m_device = device;
    for (int i = 0; i < rows; ++i)
        for (int o = 0; o < cols; ++o) refreshCell(i, o);
}

void MatrixModel::refreshCell(int input, int output) {
    const int row = input * m_cols + output;
    const Shown now{ cell(input, output), m_enabled.value(output), m_conflict.value(output) };
    if (now == m_shown.at(row)) return;
    m_shown[row] = now;
    emit dataChanged(index(row), index(row));
    ++m_version;   // -- WP MX1 --
    emit cellsChanged();
}

void MatrixModel::stage(int input, int output, const Cell &c) {
    if (!valid(input, output)) return;
    m_staged.insert(key(input, output), c);
    refreshCell(input, output);
}

void MatrixModel::unstage(int input, int output) {
    if (m_staged.remove(key(input, output)) && valid(input, output)) refreshCell(input, output);
}

void MatrixModel::unstageAll() {
    if (m_staged.isEmpty()) return;
    const QList<int> keys = m_staged.keys();
    m_staged.clear();
    for (int k : keys)
        if (valid(k / 16, k % 16)) refreshCell(k / 16, k % 16);
}
