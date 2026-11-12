// -- WP QP10 -- Settings > Overview: the GPIO map
#include "PinGrid.h"

#include <QPainter>
#include <QPainterPath>
#include <algorithm>
#include <cmath>

PinGrid::PinGrid(QQuickItem *parent) : QQuickPaintedItem(parent) {
    setAcceptHoverEvents(true);
    setAntialiasing(true);
    connect(this, &PinGrid::lookChanged, this, [this]() { update(); });
    connect(this, &QQuickItem::widthChanged, this, &PinGrid::relayout);
}

void PinGrid::setPins(const QVariantList &pins) {
    if (pins == m_pins) return;
    m_pins = pins;
    emit pinsChanged();
    relayout();
}

void PinGrid::setClaims(const QVariantList &claims) {
    if (claims == m_claims) return;
    m_claims = claims;
    m_byPin.clear();
    for (const QVariant &v : claims) {
        const QVariantMap c = v.toMap();
        m_byPin.insert(c.value(QStringLiteral("pin")).toInt(),
                       { c.value(QStringLiteral("role")).toInt(), c.value(QStringLiteral("label")).toString() });
    }
    emit claimsChanged();
    emit hoveredChanged();
    update();
}

void PinGrid::setColumns(int columns) {
    columns = std::max(columns, 1);
    if (columns == m_columns) return;
    m_columns = columns;
    emit layoutChanged();
    relayout();
}

void PinGrid::setChipHeight(qreal h) {
    if (qFuzzyCompare(h, m_chipHeight)) return;
    m_chipHeight = h;
    emit layoutChanged();
    relayout();
}

void PinGrid::setSpacing(qreal s) {
    if (qFuzzyCompare(s, m_spacing)) return;
    m_spacing = s;
    emit layoutChanged();
    relayout();
}

void PinGrid::setRoleColors(const QVariantList &colors) {
    if (colors == m_roleColors) return;
    m_roleColors = colors;
    emit lookChanged();
}

int PinGrid::heldCount() const {
    int n = 0;
    for (const QVariant &p : m_pins)
        if (m_byPin.contains(p.toInt())) ++n;
    return n;
}

void PinGrid::relayout() {
    const auto rows = (qsizetype(m_pins.size()) + m_columns - 1) / m_columns;
    setImplicitHeight(rows > 0 ? rows * m_chipHeight + (rows - 1) * m_spacing : 0);
    update();
}

QRectF PinGrid::chipRect(int pin) const {
    const auto at = m_pins.indexOf(QVariant(pin));
    if (at < 0) return {};
    const qreal w = (width() - (m_columns - 1) * m_spacing) / m_columns;
    const auto col = at % m_columns, row = at / m_columns;
    return { col * (w + m_spacing), row * (m_chipHeight + m_spacing), w, m_chipHeight };
}

int PinGrid::pinAt(qreal x, qreal y) const {
    for (const QVariant &p : m_pins)
        if (chipRect(p.toInt()).contains(x, y)) return p.toInt();
    return -1;
}

QString PinGrid::hoveredText() const {
    if (m_hoveredPin < 0) return {};
    const auto it = m_byPin.constFind(m_hoveredPin);
    if (it != m_byPin.cend()) return it->second;
    return m_freeText.arg(m_hoveredPin);
}

void PinGrid::setHovered(int pin) {
    if (pin == m_hoveredPin) return;
    m_hoveredPin = pin;
    emit hoveredChanged();
}

void PinGrid::hoverMoveEvent(QHoverEvent *event) { setHovered(pinAt(event->position().x(), event->position().y())); }

void PinGrid::hoverLeaveEvent(QHoverEvent *) { setHovered(-1); }

void PinGrid::paint(QPainter *p) {
    for (const QVariant &entry : std::as_const(m_pins)) {
        const int pin = entry.toInt();
        const QRectF r = chipRect(pin);
        const auto it = m_byPin.constFind(pin);
        const bool held = it != m_byPin.cend();
        QColor fill = m_freeColor;
        if (held && it->first >= 0 && it->first < m_roleColors.size()) fill = m_roleColors.at(it->first).value<QColor>();
        QPainterPath chip;
        chip.addRoundedRect(r.adjusted(0.5, 0.5, -0.5, -0.5), 4, 4);
        p->fillPath(chip, fill);
        QFont f = m_font;
        f.setBold(held);
        p->setFont(f);
        p->setPen(held ? m_heldTextColor : m_freeTextColor);
        p->drawText(r, Qt::AlignCenter, QString::number(pin));
    }
}
