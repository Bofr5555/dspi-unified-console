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
    // -- WP CG29 -- the links section's heading and note, which say whose links they are
    Q_PROPERTY(QString linksTitle READ linksTitle NOTIFY changed)
    Q_PROPERTY(QString linksNote READ linksNote NOTIFY changed)
    Q_PROPERTY(QVariantList credits READ credits NOTIFY changed)
    // -- WP LIC1 -- the project's licence and its section 7 terms (the table's `licence`);
    // terms are rows of {title, detail, url}, as above
    Q_PROPERTY(QString licenceTitle READ licenceTitle NOTIFY changed)
    Q_PROPERTY(QString licenceSpdx READ licenceSpdx NOTIFY changed)
    Q_PROPERTY(QStringList licenceCopyright READ licenceCopyright NOTIFY changed)
    Q_PROPERTY(QString licenceStatement READ licenceStatement NOTIFY changed)
    Q_PROPERTY(QString termsTitle READ termsTitle NOTIFY changed)
    Q_PROPERTY(QString termsNote READ termsNote NOTIFY changed)
    Q_PROPERTY(QVariantList terms READ terms NOTIFY changed)
    Q_PROPERTY(QString licenceFreedoms READ licenceFreedoms NOTIFY changed)
    Q_PROPERTY(QString licenceNotices READ licenceNotices NOTIFY changed)

public:
    explicit AboutController(DSPiBridge *bridge, QObject *parent = nullptr);

    QStringList creditLines() const { return m_creditLines; }
    QVariantList makers() const { return m_makers; }
    QVariantList links() const { return m_links; }
    QString linksTitle() const { return m_linksTitle; }
    QString linksNote() const { return m_linksNote; }
    QVariantList credits() const { return m_credits; }
    // -- WP LIC1 --
    QString licenceTitle() const { return m_licenceTitle; }
    QString licenceSpdx() const { return m_licenceSpdx; }
    QStringList licenceCopyright() const { return m_licenceCopyright; }
    QString licenceStatement() const { return m_licenceStatement; }
    QString termsTitle() const { return m_termsTitle; }
    QString termsNote() const { return m_termsNote; }
    QVariantList terms() const { return m_terms; }
    QString licenceFreedoms() const { return m_licenceFreedoms; }
    QString licenceNotices() const { return m_licenceNotices; }

    // -- WP LIC1 -- A licence text built into the app (resources.qrc, licences/),
    // by file name: GPL-3.0.txt, LGPL-3.0.txt, rust-crates.txt. Empty for any other.
    Q_INVOKABLE QString licenceText(const QString &file) const;

    // Take the table from this document. Anything but a JSON object empties it.
    // The app loads what the core holds; the smoke driver also tries other wording.
    void load(const QByteArray &json);

signals:
    void changed();

private:
    QStringList m_creditLines;
    QVariantList m_makers;
    QVariantList m_links;
    QString m_linksTitle;
    QString m_linksNote;
    QVariantList m_credits;
    // -- WP LIC1 --
    QString m_licenceTitle;
    QString m_licenceSpdx;
    QStringList m_licenceCopyright;
    QString m_licenceStatement;
    QString m_termsTitle;
    QString m_termsNote;
    QVariantList m_terms;
    QString m_licenceFreedoms;
    QString m_licenceNotices;
};

#endif // ABOUTCONTROLLER_H
