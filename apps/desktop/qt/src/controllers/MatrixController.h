#ifndef MATRIXCONTROLLER_H
#define MATRIXCONTROLLER_H

#include <QObject>
#include <functional>
#include <QString>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>

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
    // Before an enable that switches the other side of Core 1 off
    void enableQuestion(int output, const QString &title, const QString &text, const QString &acceptText);

private:
    ChannelController *channels() const;
    void onSnapshot();
    void editCell(int input, int output, const std::function<void(MatrixModel::Cell &)> &edit);

    AppContext m_ctx;
    MatrixModel *m_model;
    int m_revision = 0;
};

#endif // MATRIXCONTROLLER_H
