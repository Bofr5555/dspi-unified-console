#ifndef PEQEDITORCONTROLLER_H
#define PEQEDITORCONTROLLER_H

#include <QHash>
#include <QList>
#include <QObject>
#include <QPair>
#include <QSet>
#include <QString>
#include <QVariant>
#include <QtQml/qqmlregistration.h>
#include <memory>

#include "CoreApi.h"
#include "Registrar.h"

class QQmlEngine;
class QJSEngine;

// -- WP QP4 -- What the on-graph PEQ editor knows without a plot (the Mac's
// PeqGraphModel): the shapes it creates and their firmware types, what a
// band's dot stands for, the editor's limits, value text in and out, and the
// blend of an animated move. The response itself is always the core's
// (dspi_compute_response); nothing here evaluates a filter.
namespace Peq {

// The shapes the graph makes and offers, in the chooser's order
enum Shape { Bell, LowShelf, LowCut, HighShelf, HighCut, Notch, AllPass, ShapeCount };
enum Field { Freq, Gain, Width, FieldCount };

// The firmware type of `shape` at `order` (1 = 6 dB/oct or 180°, 2 = 12 dB/oct
// or 360°), or -1 when the shape has no such order
int typeFor(int shape, int order);
// The shape and order of a type; false for off, crossovers and the Linkwitz
// Transform, which the graph does not reshape
bool shapeOf(int type, int *shape, int *order);
QString shapeTitle(int shape);
QString shapeCode(int shape);
// The band menu's slope rows and the chooser's second step
QString orderTitle(int shape, int order);
QString orderLabel(int shape, int order);
QString orderHelp(int shape, int order);

bool isCrossover(int type);
bool usesGain(int type);
bool usesQ(int type);
// The bands the graph draws a dot for: every PEQ type but off
bool isGraphBand(const FilterParams &p);

// What a dot stands for: where it sits and what a vertical drag changes
struct Role {
    enum Kind { Gain, Resonance, Fixed, Locked } kind = Fixed;
    double value = 0;   // Gain: dB of curve per dB of gain; Fixed, Locked: the dot's level
};
Role roleOf(const FilterParams &p);
double dotDb(const Role &r, const FilterParams &p);

// The core's response of `n` bands at `hz`, in dB
double response(const FilterParams *bands, int n, double hz);
double bandDb(FilterParams p, double hz);

// Limits the graph keeps to: the firmware's Q and frequency clamps, and
// FabFilter's ±30 dB of gain
constexpr double kGainMin = -30, kGainMax = 30;
constexpr double kQMin = 0.1, kQMax = 20;
constexpr double kFreqMin = 10;
constexpr double kFreqMax = SAMPLE_RATE * 0.45;
double clamp(double v, double lo, double hi);

FilterParams flatBand();
FilterParams make(int type, double freq, double q, double gain);
// -- WP LAY3b -- `was` as a band of `type` (moved from PeqOverlay::setShape)
FilterParams reshaped(FilterParams was, int type);
bool same(const FilterParams &a, const FilterParams &b);
// A band part-way to `to`: frequency and Q in the log domain; a new type or
// bypass lands at once
FilterParams blend(const FilterParams &from, const FilterParams &to, double t);

// Value text: display, and entry with "2k" and note names ("A4", "C#2+13")
QString frequencyText(double hz);
QString shortFrequency(double hz);
QString gainText(double db);
// Two places, cut rather than rounded, so the chip never shows a value the
// band does not quite have
QString truncated(double v, bool sign = false);
bool parseFrequency(const QString &text, double *hz);
bool parseNumber(const QString &text, const QString &unit, double *v);
bool parseNote(const QString &text, double *hz);

} // namespace Peq

// The editor's state shared with the band list (the Mac's PeqGraphSelection
// and PeqLiveReadouts) and the one place its band writes go: QML sees it as
// the singleton PeqEditorController, and FilterList's `graphSelection` is
// PeqSelection.qml over it.
//
// - Selection: the bands selected on the graph or by a click on a row's
//   number, the band under the pointer on the graph (its row lights) and the
//   row under the pointer (its dot lights). A band the pointer rests on asks
//   for its row to be shown (showRow).
// - Live values: what a drag shows in the rows between device writes.
// - Writes: through ChannelController's band setters, which stage the edit,
//   mirror it onto a linked input and coalesce per band.
class PeqEditorController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

    Q_PROPERTY(int revision READ revision NOTIFY changed)
    Q_PROPERTY(QList<int> selected READ selected NOTIFY changed)
    Q_PROPERTY(int rowHovered READ rowHovered WRITE setRowHovered NOTIFY changed)
    Q_PROPERTY(int pointedBand READ pointedBand NOTIFY changed)
    // The channel the graph edits, -1 for none (set by the editor on show)
    Q_PROPERTY(int editedChannel READ editedChannel NOTIFY changed)
    // -- WP LAY3b -- a band was copied
    Q_PROPERTY(bool hasBand READ hasBand NOTIFY bandCopied)

public:
    PeqEditorController(const AppContext &ctx, QObject *parent);
    static PeqEditorController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<PeqEditorController>(); }

    int revision() const { return m_rev; }
    QList<int> selected() const;
    const QSet<int> &selection() const { return state()->selected; }
    int rowHovered() const { return m_rowHovered; }
    void setRowHovered(int band);
    int pointedBand() const { return m_pointed; }
    void setPointedBand(int band);
    int editedChannel() const { return state()->edited; }
    void setEditedChannel(int channel);

    Q_INVOKABLE bool isSelected(int band) const { return selection().contains(band); }
    // {freq, gain, q} while the graph drags the band, else null
    Q_INVOKABLE QVariant live(int band) const;
    // A click on a band's number in the list, with the modifiers of a click
    // on its dot: Ctrl toggles it, Shift takes the run of rows from the last
    // one clicked (row order, empty rows skipped), a plain click selects it
    Q_INVOKABLE void rowClicked(int band, int modifiers);
    Q_INVOKABLE void forget();

    // -- WP LAY3b -- What a band's right-click menu offers and does (ContextMenus),
    // on rows and dots alike; the edits mirror onto a linked input.
    // The shapes this firmware offers: [{shape, title}]
    Q_INVOKABLE QVariantList shapes() const;
    // A type's {shape, order}; {-1, 2} for one the graph does not reshape
    Q_INVOKABLE QVariantMap shapeOrder(int type) const;
    // Its slopes when it has two: [{name, text, value: {shape, order}, checked}]
    Q_INVOKABLE QVariantList slopes(int type) const;
    Q_INVOKABLE QString slopeTitle(int type) const;
    Q_INVOKABLE void setShape(int channel, const QList<int> &bands, int shape, int order);
    Q_INVOKABLE void bypassBands(int channel, const QList<int> &bands, bool crossover, bool bypass);
    Q_INVOKABLE void invertGain(int channel, const QList<int> &bands);
    Q_INVOKABLE void resetBands(int channel, const QList<int> &bands, bool crossover);
    // One band's own clipboard, for every device
    Q_INVOKABLE void copyBand(int channel, int band);
    Q_INVOKABLE void pasteBand(int channel, const QList<int> &bands);
    bool hasBand() const { return m_hasBand; }

    void setSelection(const QSet<int> &bands);
    void reveal(int band) { emit showRow(band); }
    void showLive(int band, const FilterParams &p);
    void endLive();

    // The channel's bands as the list shows them (a staged edit at once)
    FilterParams band(int channel, int band, bool crossover) const;
    bool writable() const;
    bool bypassSupported() const;
    QList<int> availableTypes() const;
    // Writes each changed band, mirrored onto a linked input
    void writeBands(int channel, const QList<QPair<int, FilterParams>> &changes);
    void setBandBypassed(int channel, int band, bool bypass);

signals:
    void changed();
    // The selection changed on the list's side (a row's number clicked)
    void selectionChanged();
    void rowHoveredChanged();
    void bandCopied();   // -- WP LAY3b --
    // The list should scroll this band's row into view
    void showRow(int band);

private:
    // -- WP MDQT2a -- The channel edited and its selection are the device's own
    struct State {
        int edited = -1;
        int selectedOn = -1;   // the channel `selected` belongs to
        QSet<int> selected;
        int rowAnchor = -1;
    };
    std::shared_ptr<State> state() const;
    // The pointer and a drag's live values stay behind with the device left
    void letGo();
    void bump();

    AppContext m_ctx;
    // A switch is under way: the pages move to the next device's channels
    bool m_switching = false;
    int m_rowHovered = -1;
    int m_pointed = -1;
    QHash<int, FilterParams> m_live;
    FilterParams m_copied{};   // -- WP LAY3b --
    bool m_hasBand = false;
    int m_rev = 0;
};

#endif // PEQEDITORCONTROLLER_H
