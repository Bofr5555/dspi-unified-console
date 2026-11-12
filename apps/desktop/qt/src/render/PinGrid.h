#ifndef PINGRID_H
#define PINGRID_H

#include <QColor>
#include <QFont>
#include <QHash>
#include <QQuickPaintedItem>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

// Settings > Overview's map of the header: every valid GPIO in pin order, a
// fixed number of columns, a held pin filled with its role's colour and a
// free one a muted outline of the same shape, so occupancy reads as colour
// (the Mac's summary map). The QML sets the colours from Theme; hovering a chip
// sets `hoveredPin` and `hoveredText` for the tooltip.
class PinGrid : public QQuickPaintedItem {
    Q_OBJECT
    QML_ELEMENT
    // Every pin shown, in order (SettingsController.gpioPins)
    Q_PROPERTY(QVariantList pins READ pins WRITE setPins NOTIFY pinsChanged)
    // [{pin, role, label}] (SettingsController.pinClaims)
    Q_PROPERTY(QVariantList claims READ claims WRITE setClaims NOTIFY claimsChanged)
    Q_PROPERTY(int columns READ columns WRITE setColumns NOTIFY layoutChanged)
    Q_PROPERTY(qreal chipHeight READ chipHeight WRITE setChipHeight NOTIFY layoutChanged)
    Q_PROPERTY(qreal spacing READ spacing WRITE setSpacing NOTIFY layoutChanged)
    // One colour per role, in role order
    Q_PROPERTY(QVariantList roleColors READ roleColors WRITE setRoleColors NOTIFY lookChanged)
    Q_PROPERTY(QColor freeColor MEMBER m_freeColor NOTIFY lookChanged)
    Q_PROPERTY(QColor heldTextColor MEMBER m_heldTextColor NOTIFY lookChanged)
    Q_PROPERTY(QColor freeTextColor MEMBER m_freeTextColor NOTIFY lookChanged)
    Q_PROPERTY(QFont font MEMBER m_font NOTIFY lookChanged)
    // The text a free pin's tooltip shows, with %1 for its number
    Q_PROPERTY(QString freeText MEMBER m_freeText NOTIFY lookChanged)
    Q_PROPERTY(int hoveredPin READ hoveredPin NOTIFY hoveredChanged)
    Q_PROPERTY(QString hoveredText READ hoveredText NOTIFY hoveredChanged)
    Q_PROPERTY(int heldCount READ heldCount NOTIFY claimsChanged)
public:
    explicit PinGrid(QQuickItem *parent = nullptr);

    QVariantList pins() const { return m_pins; }
    void setPins(const QVariantList &pins);
    QVariantList claims() const { return m_claims; }
    void setClaims(const QVariantList &claims);
    int columns() const { return m_columns; }
    void setColumns(int columns);
    qreal chipHeight() const { return m_chipHeight; }
    void setChipHeight(qreal h);
    qreal spacing() const { return m_spacing; }
    void setSpacing(qreal s);
    QVariantList roleColors() const { return m_roleColors; }
    void setRoleColors(const QVariantList &colors);
    int hoveredPin() const { return m_hoveredPin; }
    QString hoveredText() const;
    int heldCount() const;

    // The pin under `x, y` (item coordinates), -1 for none
    Q_INVOKABLE int pinAt(qreal x, qreal y) const;
    // The chip of `pin` (item coordinates); empty for a pin not shown
    Q_INVOKABLE QRectF chipRect(int pin) const;

    void paint(QPainter *p) override;

signals:
    void pinsChanged();
    void claimsChanged();
    void layoutChanged();
    void lookChanged();
    void hoveredChanged();

protected:
    void hoverMoveEvent(QHoverEvent *event) override;
    void hoverLeaveEvent(QHoverEvent *event) override;

private:
    void relayout();
    void setHovered(int pin);

    QVariantList m_pins;
    QVariantList m_claims;
    QHash<int, QPair<int, QString>> m_byPin;   // pin -> role, label
    QVariantList m_roleColors;
    int m_columns = 9;
    qreal m_chipHeight = 20;
    qreal m_spacing = 4;
    QColor m_freeColor;
    QColor m_heldTextColor;
    QColor m_freeTextColor;
    QFont m_font;
    QString m_freeText;
    int m_hoveredPin = -1;
};

#endif // PINGRID_H
