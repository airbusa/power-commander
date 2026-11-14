// sc-gtkstyle: the desktop's GTK 3 theme (Xfce's, MATE's, Cinnamon's) draws
// SparkCommander's controls, each in every state, into images on stdout, as
// sc-qtstyle does with Qt's, and says its colours:
//   sc-gtkstyle <scale> <button height> | sc-gtkstyle colors
// Header lines, "color <name> <rrggbb>" lines, then per image
// "image <name> <width> <height>" and its straight RGBA pixels; sizes in
// device pixels, "end" last.

#include <gtk/gtk.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static double scale = 1;

// The pixels of `surface` from column `x`, `width` wide (device pixels).
static void put(const char *name, cairo_surface_t *surface, int x, int width) {
    cairo_surface_flush(surface);
    int height = cairo_image_surface_get_height(surface);
    int stride = cairo_image_surface_get_stride(surface);
    const unsigned char *data = cairo_image_surface_get_data(surface);
    printf("image %s %d %d\n", name, width, height);
    fflush(stdout);
    unsigned char *row = g_malloc(width * 4);
    for (int y = 0; y < height; y++) {
        const guint32 *pixels = (const guint32 *)(data + y * stride) + x;
        for (int i = 0; i < width; i++) {
            // Cairo's are premultiplied ARGB in a word.
            guint32 p = pixels[i];
            unsigned a = p >> 24, r = p >> 16 & 255, g = p >> 8 & 255, b = p & 255;
            if (a) {
                r = r * 255 / a, g = g * 255 / a, b = b * 255 / a;
            }
            unsigned char *out = row + i * 4;
            out[0] = r, out[1] = g, out[2] = b, out[3] = a;
        }
        fwrite(row, 1, width * 4, stdout);
    }
    g_free(row);
}

// A CSS node under `parent`, as GTK builds a widget's: so the theme's rules
// for it, and for where it is, apply.
static GtkStyleContext *node(GtkStyleContext *parent, GType type, const char *name, const char *klass) {
    GtkWidgetPath *path = parent ? gtk_widget_path_copy(gtk_style_context_get_path(parent)) : gtk_widget_path_new();
    gtk_widget_path_append_type(path, type);
    gtk_widget_path_iter_set_object_name(path, -1, name);
    if (klass) gtk_widget_path_iter_add_class(path, -1, klass);
    GtkStyleContext *context = gtk_style_context_new();
    gtk_style_context_set_path(context, path);
    gtk_style_context_set_parent(context, parent);
    gtk_widget_path_unref(path);
    return context;
}

enum { NORMAL, HOVERED, PRESSED, DISABLED };
static const char *const STATES[] = {"normal", "hovered", "pressed", "disabled"};

static GtkStateFlags flags(int state, gboolean on) {
    GtkStateFlags flags = state == HOVERED  ? GTK_STATE_FLAG_PRELIGHT
                          : state == PRESSED ? GTK_STATE_FLAG_PRELIGHT | GTK_STATE_FLAG_ACTIVE
                          : state == DISABLED ? GTK_STATE_FLAG_INSENSITIVE
                                              : GTK_STATE_FLAG_NORMAL;
    return on ? flags | GTK_STATE_FLAG_CHECKED : flags;
}

// An image `width` × `height` logical pixels at the scale, cleared.
static cairo_surface_t *canvas(int width, int height, cairo_t **cr) {
    cairo_surface_t *surface = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, width * scale + 0.5, height * scale + 0.5);
    cairo_surface_set_device_scale(surface, scale, scale);
    *cr = cairo_create(surface);
    return surface;
}

// `context`'s box drawn in `width` × `height`; a check box's mark too, inside
// its border and padding, as GTK does. Its margins, the space around it, are
// the app's to lay out.
static void draw(GtkStyleContext *context, cairo_t *cr, double width, double height, gboolean check) {
    gtk_render_background(context, cr, 0, 0, width, height);
    gtk_render_frame(context, cr, 0, 0, width, height);
    if (!check) return;
    GtkStateFlags state = gtk_style_context_get_state(context);
    GtkBorder padding, border;
    gtk_style_context_get_padding(context, state, &padding);
    gtk_style_context_get_border(context, state, &border);
    double left = padding.left + border.left, top = padding.top + border.top;
    gtk_render_check(context, cr, left, top, width - left - padding.right - border.right, height - top - padding.bottom - border.bottom);
}

// A node's size: its min-width and min-height with padding and border.
static void size_of(GtkStyleContext *context, int *width, int *height) {
    GtkStateFlags state = gtk_style_context_get_state(context);
    GtkBorder padding, border;
    gtk_style_context_get(context, state, "min-width", width, "min-height", height, NULL);
    gtk_style_context_get_padding(context, state, &padding);
    gtk_style_context_get_border(context, state, &border);
    *width += padding.left + padding.right + border.left + border.right;
    *height += padding.top + padding.bottom + border.top + border.bottom;
}

// A colour over the window's, as an opaque "rrggbb".
static void put_color(const char *name, GdkRGBA color, GdkRGBA under) {
    double a = color.alpha;
    int r = (color.red * a + under.red * (1 - a)) * 255 + 0.5;
    int g = (color.green * a + under.green * (1 - a)) * 255 + 0.5;
    int b = (color.blue * a + under.blue * (1 - a)) * 255 + 0.5;
    printf("color %s %02x%02x%02x\n", name, r, g, b);
}

int main(int argc, char **argv) {
    if (!gtk_init_check(&argc, &argv)) {
        fprintf(stderr, "sc-gtkstyle: no display\n");
        return 1;
    }
    gboolean colors_only = argc == 2 && !strcmp(argv[1], "colors");
    if (argc < 3 && !colors_only) {
        fprintf(stderr, "usage: sc-gtkstyle <scale> <button height> | sc-gtkstyle colors\n");
        return 2;
    }
    scale = colors_only ? 1 : g_ascii_strtod(argv[1], NULL);
    int button_height = colors_only ? 24 : atoi(argv[2]);
    if (scale <= 0 || button_height <= 0) return 2;

    // GTK_THEME, when set, wins over the settings' theme.
    gchar *theme = g_strdup(g_getenv("GTK_THEME"));
    if (!theme) g_object_get(gtk_settings_get_default(), "gtk-theme-name", &theme, NULL);
    GtkStyleContext *window = node(NULL, GTK_TYPE_WINDOW, "window", "background");
    GdkRGBA background = {1, 1, 1, 1};
    gtk_style_context_lookup_color(window, "theme_bg_color", &background);
    double lightness = 0.299 * background.red + 0.587 * background.green + 0.114 * background.blue;
    printf("sc-gtkstyle 1\nstyle %s\ndark %d\n", theme ? theme : "", lightness < 0.5);
    g_free(theme);

    // The named colours GTK's apps paint with, where the theme defines them.
    static const char *const COLORS[] = {"theme_bg_color", "theme_fg_color", "theme_base_color", "theme_text_color",
                                         "theme_selected_bg_color", "theme_selected_fg_color", "borders", "error_color"};
    for (size_t i = 0; i < G_N_ELEMENTS(COLORS); i++) {
        GdkRGBA color;
        if (gtk_style_context_lookup_color(window, COLORS[i], &color)) put_color(COLORS[i], color, background);
    }
    // A push button's fill, its middle as drawn: themes paint gradients and images.
    GtkStyleContext *plain = node(window, GTK_TYPE_BUTTON, "button", "text-button");
    cairo_t *cr;
    cairo_surface_t *surface = canvas(button_height, button_height, &cr);
    draw(plain, cr, button_height, button_height, FALSE);
    cairo_destroy(cr);
    cairo_surface_flush(surface);
    int x = cairo_image_surface_get_width(surface) / 2, y = cairo_image_surface_get_height(surface) / 2;
    guint32 middle = *(guint32 *)(cairo_image_surface_get_data(surface) + cairo_image_surface_get_stride(surface) * y + x * 4);
    double alpha = (middle >> 24) / 255.;
    if (alpha > 0) {
        GdkRGBA fill = {(middle >> 16 & 255) / 255. / alpha, (middle >> 8 & 255) / 255. / alpha, (middle & 255) / 255. / alpha, alpha};
        put_color("button", fill, background);
    }
    cairo_surface_destroy(surface);
    g_object_unref(plain);
    if (colors_only) {
        printf("end\n");
        return 0;
    }

    GtkStyleContext *checkbutton = node(window, GTK_TYPE_CHECK_BUTTON, "checkbutton", NULL);
    GtkStyleContext *check = node(checkbutton, G_TYPE_NONE, "check", NULL);
    int width, height;
    size_of(check, &width, &height);
    for (int on = 0; on <= 1; on++) {
        for (int state = NORMAL; state <= DISABLED; state++) {
            gtk_style_context_set_state(checkbutton, flags(state, on));
            gtk_style_context_set_state(check, flags(state, on));
            cairo_t *cr;
            cairo_surface_t *surface = canvas(width, height, &cr);
            draw(check, cr, width, height, TRUE);
            cairo_destroy(cr);
            char name[64];
            snprintf(name, sizeof name, "checkbox.%s.%s", on ? "on" : "off", STATES[state]);
            put(name, surface, 0, cairo_image_surface_get_width(surface));
            cairo_surface_destroy(surface);
        }
    }

    // A button four times as wide as high, cut into its ends (half its height
    // each) and a strip of its middle, which the app stretches to any width.
    for (int is_default = 0; is_default <= 1; is_default++) {
        GtkStyleContext *button = node(window, GTK_TYPE_BUTTON, "button", "text-button");
        if (is_default) gtk_style_context_add_class(button, GTK_STYLE_CLASS_DEFAULT);
        for (int state = NORMAL; state <= DISABLED; state++) {
            gtk_style_context_set_state(button, flags(state, FALSE));
            cairo_t *cr;
            cairo_surface_t *surface = canvas(button_height * 4, button_height, &cr);
            draw(button, cr, button_height * 4, button_height, FALSE);
            cairo_destroy(cr);
            int full = cairo_image_surface_get_width(surface), end = cairo_image_surface_get_height(surface) / 2, strip = 4;
            char name[64];
            const char *kind = is_default ? "default" : "plain";
            snprintf(name, sizeof name, "button.%s.%s.left", kind, STATES[state]);
            put(name, surface, 0, end);
            snprintf(name, sizeof name, "button.%s.%s.middle", kind, STATES[state]);
            put(name, surface, full / 2 - strip / 2, strip);
            snprintf(name, sizeof name, "button.%s.%s.right", kind, STATES[state]);
            put(name, surface, full - end, end);
            cairo_surface_destroy(surface);
        }
        g_object_unref(button);
    }
    printf("end\n");
    return 0;
}
