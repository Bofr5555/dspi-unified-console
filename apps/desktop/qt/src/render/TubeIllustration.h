#ifndef TUBEILLUSTRATION_H
#define TUBEILLUSTRATION_H

#include <QQuickPaintedItem>
#include <QtQml/qqmlregistration.h>

// -- WP QP8 -- One layer of the Tube Modeller's tube (the Mac's
// TubeIllustration): the glass, plates, base and pins (`part` Tube), the heater
// glow (Heater) or the flare laid over it while audio passes (Bloom). The
// window stacks the three and fades the heater with the switch and the bloom
// with the outputs' level, so nothing here repaints for audio: a layer paints
// only when its family or size changes.
//
// The tubes are drawn on a 120 x 200 grid from one geometry per family
// (ToolsController::tubeFamily), centred and scaled to fit. The artwork's
// colours (glass, steel, bakelite, the heater's orange) are the drawing's own
// and read the same on a light or a dark panel.
class TubeIllustration : public QQuickPaintedItem {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(int family READ family WRITE setFamily NOTIFY familyChanged)
    Q_PROPERTY(Part part READ part WRITE setPart NOTIFY partChanged)
public:
    enum Part { Tube, Heater, Bloom };
    Q_ENUM(Part)
    // The drawing families, as ToolsController::tubeFamily numbers them
    static constexpr int kFamilies = 10;

    explicit TubeIllustration(QQuickItem *parent = nullptr);

    int family() const { return m_family; }
    void setFamily(int family);
    Part part() const { return m_part; }
    void setPart(Part part);

    void paint(QPainter *painter) override;

signals:
    void familyChanged();
    void partChanged();

private:
    int m_family = 0;
    Part m_part = Tube;
};

#endif // TUBEILLUSTRATION_H
