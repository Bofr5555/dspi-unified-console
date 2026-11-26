#ifndef MATRIXCONTROLLER_H
#define MATRIXCONTROLLER_H

#include <QObject>
#include <functional>
#include <QString>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <memory>

#include "MatrixModel.h"
#include "Registrar.h"

class ChannelController;
class QJSEngine;
class QQmlEngine;

// -- WP QP6 -- The matrix mixer window's controller (the Mac's MatrixMixerView):
// the crosspoints (MatrixModel), the per-output ENABLE / GAIN / DELAY / MUTE
// row, the input trims of the 8-channel layout, Direct 1:1 and Clear, and the
// column header's rename.
//
// One writer per target, shared with the output card (QP2):
// - a crosspoint, an output's enable, gain, delay and mute and an input's trim
//   are written through ChannelController (keys "matrix route <i> <o>",
//   "output enable <o>", "output gain <o>", ...), so an edit in the matrix and
//   one on the output card coalesce into one write and both read the same
//   snapshot;
// - Direct 1:1 and Clear touch many crosspoints at once and run as one write
//   each ("matrix routes"), on the same worker queue, so they land in order
//   with the single edits around them.
// The window reads only this controller and its model; nothing here keeps a
// copy of the device's state beyond the staged crosspoints (MatrixModel).
//
// Source rows: the inputs the sidebar shows, or, while the upmixer derives
// rows from a stereo input, 3 (L, R, Upmix Centre) or 5 (and the two
// surrounds) rows.
class MatrixController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(MatrixModel *model READ model CONSTANT)
    // Bumped on every applied snapshot: bindings calling column()/row() name it
    Q_PROPERTY(int revision READ revision NOTIFY changed)
    Q_PROPERTY(bool writable READ writable NOTIFY changed)
    Q_PROPERTY(int rows READ rows NOTIFY changed)
    Q_PROPERTY(int columns READ columns NOTIFY changed)
    Q_PROPERTY(int pdmOutput READ pdmOutput NOTIFY changed)
    // The 8-channel layout (an RP2350 on the current wire format with eight
    // inputs): input trims, Direct 1:1 and Clear, and a scrolling table
    Q_PROPERTY(bool wide READ wide NOTIFY changed)
    // The upmixer turns a stereo input into 3 or 5 source rows
    Q_PROPERTY(bool upmixRows READ upmixRows NOTIFY changed)
    Q_PROPERTY(double maxDelayMs READ maxDelayMs NOTIFY changed)
    // -- WP MX1 -- Link L/R pairs: an edit of a crosspoint of a stereo pair's
    // input is mirrored onto the pair's other input one column over, inside
    // the output pair (L->Out 1 and R->Out 2, L->Out 2 and R->Out 1), as one
    // undo step. Off by default, kept per device (its serial).
    Q_PROPERTY(bool linkPairs READ linkPairs WRITE setLinkPairs NOTIFY viewChanged)
    // -- WP MX1 -- The dense grid, chosen (it is also used past 12 outputs)
    Q_PROPERTY(bool dense READ dense WRITE setDense NOTIFY viewChanged)
    // -- WP MX1 -- The crosspoint the grid's keyboard focus and the inspector
    // are on, -1 / -1 for none
    Q_PROPERTY(int currentInput READ currentInput NOTIFY currentChanged)
    Q_PROPERTY(int currentOutput READ currentOutput NOTIFY currentChanged)
public:
    MatrixController(const AppContext &ctx, QObject *parent);
    static MatrixController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<MatrixController>(); }

    // The Mac's compact fields: gains from -60 to +12 dB
    static constexpr double GainMin = -60.0;
    static constexpr double GainMax = 12.0;
    // -- WP QPI2 -- an output's gain: the device's range, as the output card's
    static constexpr double OutputGainMax = 20.0;

    MatrixModel *model() const { return m_model; }
    int revision() const { return m_revision; }
    bool writable() const;
    int rows() const { return m_model->rows(); }
    int columns() const { return m_model->columns(); }
    int pdmOutput() const;
    bool wide() const;
    bool upmixRows() const;
    double maxDelayMs() const;
    // -- WP MX1 --
    bool linkPairs() const { return m_linkPairs; }
    void setLinkPairs(bool on);
    bool dense() const { return m_dense; }
    void setDense(bool on);
    int currentInput() const { return m_currentInput; }
    int currentOutput() const { return m_currentOutput; }

    // Source rows for `inputs` inputs: the same, or 3 / 5 while the upmixer
    // derives rows from a stereo input (surround engine off / on)
    static int sourceRows(int inputs, bool upmixDerives, bool surroundOff);

    // ---- Reading -----------------------------------------------------------
    // {name, fullName, input (a real input, or -1 for an upmix row), trim}
    Q_INVOKABLE QVariantMap row(int row) const;
    // {channel, label, name, descriptor, pdm, enabled, muted, gain, delay,
    //  conflict, enableTip}
    Q_INVOKABLE QVariantMap column(int output) const;

    // ---- Crosspoints -------------------------------------------------------
    Q_INVOKABLE void setConnected(int input, int output, bool connected);
    Q_INVOKABLE void setGain(int input, int output, double db);
    Q_INVOKABLE void setInvert(int input, int output, bool invert);
    // -- WP MX1 -- The whole crosspoint at once (mirrored as the others are)
    Q_INVOKABLE void setRoute(int input, int output, bool connected, double gain, bool invert);
    // The source row linked with `row` (its stereo partner, both real inputs),
    // -1 for none; whether or not Link L/R pairs is on
    Q_INVOKABLE int pairRow(int row) const;
    // The crosspoint an edit of (input, output) is mirrored to while Link L/R
    // pairs is on: {input, output}, empty for none
    Q_INVOKABLE QVariantMap mirrorOf(int input, int output) const;
    // The crosspoint the grid is on; out of range for none
    Q_INVOKABLE void setCurrent(int input, int output);
    // Each input to the matching output at 0 dB, every other crosspoint off,
    // those outputs on and the PDM sub off (the Mac's applyDirectRouting)
    Q_INVOKABLE void directRouting();
    // Every crosspoint off; gains and polarity stay
    Q_INVOKABLE void clearRoutes();

    // ---- Outputs -----------------------------------------------------------
    // ENABLE: switching off never asks; switching on may need the PDM / Core 1
    // question first (enableQuestion, then confirmEnable)
    Q_INVOKABLE void toggleEnable(int output);
    Q_INVOKABLE void confirmEnable(int output);
    Q_INVOKABLE void setOutputGain(int output, double db);
    Q_INVOKABLE void setOutputDelay(int output, double ms);
    Q_INVOKABLE void toggleMute(int output);
    // The column header's rename: trimmed, an empty name changes nothing
    Q_INVOKABLE bool renameOutput(int output, const QString &name);

    // ---- Inputs ------------------------------------------------------------
    // The trim (preamp) of input `input`, mirrored onto a linked partner
    Q_INVOKABLE void setTrim(int input, double db);

signals:
    void changed();
    void viewChanged();      // -- WP MX1 --
    void currentChanged();   // -- WP MX1 --
    // Before an enable that switches the other side of Core 1 off
    void enableQuestion(int output, const QString &title, const QString &text, const QString &acceptText);

private:
    // -- WP MDQT2a -- A device's staged crosspoints while another one shows
    struct State {
        QHash<int, MatrixModel::Cell> staged;
    };
    ChannelController *channels() const;
    void onSnapshot();
    void editCell(int input, int output, const std::function<void(MatrixModel::Cell &)> &edit);

    AppContext m_ctx;
    MatrixModel *m_model;
    std::shared_ptr<State> m_shown;   // -- WP MDQT2a -- of the device the model shows
    int m_revision = 0;
    // -- WP MX1 --
    void loadLinkPairs();
    QString m_linkSerial;
    bool m_linkPairs = false;
    bool m_dense = false;
    int m_currentInput = -1;
    int m_currentOutput = -1;
};

#endif // MATRIXCONTROLLER_H
