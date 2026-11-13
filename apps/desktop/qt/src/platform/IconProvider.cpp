#include "IconProvider.h"

#include <QColor>
#include <QFile>
#include <QPainter>
#include <QRegularExpression>
#include <QSvgRenderer>

namespace {
const char *const kRoot = ":/qt/qml/UConsole/icons/";

QByteArray iconSvg(const QString &name) {
    // Names are Lucide's: lower case, digits and dashes; nothing else reaches a path
    static const QRegularExpression valid(QStringLiteral("^[a-z0-9-]+$"));
    if (!valid.match(name).hasMatch()) return QByteArray();
    for (const char *dir : { "lucide/", "own/" }) {
        QFile f(QLatin1String(kRoot) + QLatin1String(dir) + name + QLatin1String(".svg"));
        if (f.open(QIODevice::ReadOnly)) return f.readAll();
    }
    return QByteArray();
}
} // namespace

IconProvider::IconProvider() : QQuickImageProvider(QQuickImageProvider::Image) {}

QImage IconProvider::requestImage(const QString &id, QSize *size, const QSize &requestedSize) {
    const QString name = id.section(QLatin1Char('/'), 0, 0);
    const QColor color(QLatin1Char('#') + id.section(QLatin1Char('/'), 1, 1));
    const QSize out = requestedSize.isValid() && !requestedSize.isEmpty() ? requestedSize : QSize(24, 24);
    if (size) *size = out;

    QImage image(out, QImage::Format_ARGB32_Premultiplied);
    image.fill(Qt::transparent);
    QByteArray svg = iconSvg(name);
    if (svg.isEmpty()) {
        qWarning("IconProvider: no icon \"%s\"", qPrintable(name));
        return image;
    }
    const QColor c = color.isValid() ? color : QColor(Qt::black);
    svg.replace("currentColor", c.name(QColor::HexRgb).toLatin1());
    QSvgRenderer renderer(svg);
    QPainter p(&image);
    p.setRenderHint(QPainter::Antialiasing);
    if (c.alphaF() < 1.0) p.setOpacity(c.alphaF());
    renderer.render(&p);
    return image;
}
