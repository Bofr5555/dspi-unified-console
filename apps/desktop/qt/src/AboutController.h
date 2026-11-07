#ifndef ABOUTCONTROLLER_H
#define ABOUTCONTROLLER_H

#include <QByteArray>
#include <QObject>
#include <QStringList>
#include <QVariantList>

class DSPiBridge;

// The About page's creator info, from the core's about table (dspi_about_json,
// assets/about.json), so this app keeps no copy of it. Read once: it is built into
// the library, and reading it takes no device and doesn't block.
//
// Every row is {title, detail, url}, each always a string, so QML never meets a
// field the table left out. url is empty for a row with no link.
class AboutController : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QStringList creditLines READ creditLines NOTIFY changed)
    Q_PROPERTY(QVariantList makers READ makers NOTIFY changed)
    Q_PROPERTY(QVariantList links READ links NOTIFY changed)
    Q_PROPERTY(QVariantList credits READ credits NOTIFY changed)

public:
    explicit AboutController(DSPiBridge *bridge, QObject *parent = nullptr);

    QStringList creditLines() const { return m_creditLines; }
    QVariantList makers() const { return m_makers; }
    QVariantList links() const { return m_links; }
    QVariantList credits() const { return m_credits; }

    // Take the table from this document. Anything but a JSON object empties it.
    // The app loads what the core holds; the smoke driver also tries other wording.
    void load(const QByteArray &json);

signals:
    void changed();

private:
    QStringList m_creditLines;
    QVariantList m_makers;
    QVariantList m_links;
    QVariantList m_credits;
};

#endif // ABOUTCONTROLLER_H
