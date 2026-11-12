#ifndef ICONPROVIDER_H
#define ICONPROVIDER_H

#include <QQuickImageProvider>

// "image://icon/<name>/<color>": the icon's SVG (icons/lucide/<name>.svg, or
// icons/own/<name>.svg) with currentColor set to <color> (rrggbb or aarrggbb,
// no '#'), drawn at the requested size. One SVG serves both colour schemes;
// QML uses it through the Icon component.
class IconProvider : public QQuickImageProvider {
public:
    IconProvider();
    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override;
};

#endif // ICONPROVIDER_H
