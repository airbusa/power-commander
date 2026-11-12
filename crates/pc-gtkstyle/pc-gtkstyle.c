// pc-gtkstyle: the desktop's GTK 3 theme (Xfce's, MATE's, Cinnamon's) draws
// PowerCommander's controls, each in every state, into images on stdout, as
// pc-qtstyle does with Qt's:
//   pc-gtkstyle <scale> <button height>
// Header lines, then per image "image <name> <width> <height>" and its
// straight RGBA pixels; sizes in device pixels, "end" last.

#include <gtk/gtk.h>
#include <stdio.h>
#include <stdlib.h>

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

// `context`'s box drawn in `width` × `height`, inside its margins; a check
// box's mark too.
static void draw(GtkStyleContext *context, cairo_t *cr, double width, double height, gboolean check) {
    GtkBorder margin;
    gtk_style_context_get_margin(context, gtk_style_context_get_state(context), &margin);
    double x = margin.left, y = margin.top;
    width -= margin.left + margin.right, height -= margin.top + margin.bottom;
    gtk_render_background(context, cr, x, y, width, height);
    gtk_render_frame(context, cr, x, y, width, height);
    if (check) gtk_render_check(context, cr, x, y, width, height);
}

// A node's size: its min-width and min-height with padding, border and margin.
static void size_of(GtkStyleContext *context, int *width, int *height) {
    GtkStateFlags state = gtk_style_context_get_state(context);
    GtkBorder padding, border, margin;
    gtk_style_context_get(context, state, "min-width", width, "min-height", height, NULL);
    gtk_style_context_get_padding(context, state, &padding);
    gtk_style_context_get_border(context, state, &border);
    gtk_style_context_get_margin(context, state, &margin);
    *width += padding.left + padding.right + border.left + border.right + margin.left + margin.right;
    *height += padding.top + padding.bottom + border.top + border.bottom + margin.top + margin.bottom;
}

int main(int argc, char **argv) {
    if (!gtk_init_check(&argc, &argv)) {
        fprintf(stderr, "pc-gtkstyle: no display\n");
        return 1;
    }
    if (argc < 3) {
        fprintf(stderr, "usage: pc-gtkstyle <scale> <button height>\n");
        return 2;
    }
    scale = g_ascii_strtod(argv[1], NULL);
    int button_height = atoi(argv[2]);
    if (scale <= 0 || button_height <= 0) return 2;

    // GTK_THEME, when set, wins over the settings' theme.
    gchar *theme = g_strdup(g_getenv("GTK_THEME"));
    if (!theme) g_object_get(gtk_settings_get_default(), "gtk-theme-name", &theme, NULL);
    GtkStyleContext *window = node(NULL, GTK_TYPE_WINDOW, "window", "background");
    GdkRGBA background = {1, 1, 1, 1};
    gtk_style_context_lookup_color(window, "theme_bg_color", &background);
    double lightness = 0.299 * background.red + 0.587 * background.green + 0.114 * background.blue;
    printf("pc-gtkstyle 1\nstyle %s\ndark %d\n", theme ? theme : "", lightness < 0.5);
    g_free(theme);

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
