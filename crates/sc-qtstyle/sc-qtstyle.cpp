// sc-qtstyle: the desktop's Qt style (KDE's Application Style) draws
// SparkCommander's controls, each in every state, into images on stdout:
//   sc-qtstyle <scale> <button height>
// Header lines, then per image "image <name> <width> <height>" and its
// straight RGBA pixels; sizes in device pixels, "end" last.

#include <QApplication>
#include <QImage>
#include <QPainter>
#include <QStyle>
#include <QStyleOptionButton>
#include <cstdio>

static double scale = 1;

static void put(const QByteArray &name, const QImage &drawn) {
    QImage image = drawn.convertToFormat(QImage::Format_RGBA8888);
    std::printf("image %s %d %d\n", name.constData(), image.width(), image.height());
    std::fflush(stdout);
    // 4 bytes a pixel: no padding at the ends of lines.
    std::fwrite(image.constBits(), 1, image.sizeInBytes(), stdout);
}

// A control `size` logical pixels large, drawn by `draw`.
template <typename Draw> static QImage drawn(QSize size, Draw draw) {
    QImage image(size * scale, QImage::Format_ARGB32_Premultiplied);
    image.setDevicePixelRatio(scale);
    image.fill(Qt::transparent);
    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing);
    draw(painter);
    painter.end();
    return image;
}

enum State { Normal, Hovered, Pressed, Disabled };
static const char *const STATES[] = {"normal", "hovered", "pressed", "disabled"};

// As a widget in an active window sets it.
static void set_state(QStyleOption &option, State state, const QPalette &palette) {
    option.palette = palette;
    option.direction = Qt::LeftToRight;
    option.state = QStyle::State_Active;
    if (state == Disabled) {
        option.palette.setCurrentColorGroup(QPalette::Disabled);
    } else {
        option.palette.setCurrentColorGroup(QPalette::Active);
        option.state |= QStyle::State_Enabled;
    }
    if (state == Hovered) option.state |= QStyle::State_MouseOver;
}

int main(int argc, char **argv) {
    qputenv("QT_QPA_PLATFORM", "offscreen");
    // The KDE platform theme reads the user's style, colours and fonts.
    if (qEnvironmentVariableIsEmpty("QT_QPA_PLATFORMTHEME")) qputenv("QT_QPA_PLATFORMTHEME", "kde");
    QApplication app(argc, argv);
    if (argc < 3) {
        std::fprintf(stderr, "usage: sc-qtstyle <scale> <button height>\n");
        return 2;
    }
    // Qt has set the user's locale: a decimal comma would cut "1.25" short.
    scale = QByteArray(argv[1]).toDouble();
    int button_height = QByteArray(argv[2]).toInt();
    if (scale <= 0 || button_height <= 0) return 2;
    QStyle *style = app.style();
    QPalette palette = app.palette();

    std::printf("sc-qtstyle 1\nstyle %s\n", qPrintable(style->name()));
    std::printf("dark %d\n", palette.color(QPalette::Window).lightness() < 128 ? 1 : 0);
    QSize indicator(style->pixelMetric(QStyle::PM_IndicatorWidth), style->pixelMetric(QStyle::PM_IndicatorHeight));
    std::printf("indicator %d %d\n", indicator.width(), indicator.height());

    for (bool on : {false, true}) {
        for (State state : {Normal, Hovered, Pressed, Disabled}) {
            QStyleOptionButton option;
            set_state(option, state, palette);
            option.state |= on ? QStyle::State_On : QStyle::State_Off;
            if (state == Pressed) option.state |= QStyle::State_Sunken;
            option.rect = QRect(QPoint(), indicator);
            QImage image = drawn(indicator, [&](QPainter &painter) {
                style->drawPrimitive(QStyle::PE_IndicatorCheckBox, &option, &painter, nullptr);
            });
            put(QByteArray("checkbox.") + (on ? "on." : "off.") + STATES[state], image);
        }
    }

    // A button four times as wide as high, cut into its ends (half its height
    // each) and a strip of its middle, which the app stretches to any width.
    QSize button(button_height * 4, button_height);
    for (bool is_default : {false, true}) {
        for (State state : {Normal, Hovered, Pressed, Disabled}) {
            QStyleOptionButton option;
            set_state(option, state, palette);
            option.state |= state == Pressed ? QStyle::State_Sunken : QStyle::State_Raised;
            if (is_default) option.features |= QStyleOptionButton::DefaultButton;
            option.rect = QRect(QPoint(), button);
            QImage image = drawn(button, [&](QPainter &painter) {
                style->drawControl(QStyle::CE_PushButtonBevel, &option, &painter, nullptr);
            });
            int width = image.width(), height = image.height(), end = height / 2, strip = 4;
            QByteArray name = QByteArray("button.") + (is_default ? "default." : "plain.") + STATES[state];
            put(name + ".left", image.copy(0, 0, end, height));
            put(name + ".middle", image.copy(width / 2 - strip / 2, 0, strip, height));
            put(name + ".right", image.copy(width - end, 0, end, height));
        }
    }
    std::printf("end\n");
    return 0;
}
