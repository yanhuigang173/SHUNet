package com.shunet.android;

import android.graphics.Canvas;
import android.graphics.ColorFilter;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.PixelFormat;
import android.graphics.RectF;
import android.graphics.drawable.Drawable;

/** Small code-native vector icons; no icon font or bitmap dependencies. */
final class UiIcon extends Drawable {
    enum Kind { WIFI, REFRESH, CHECK, USER, CHEVRON, EYE, EYE_OFF, LOGOUT, HISTORY, HELP, GLOBE, PLUS, MORE, OPEN }
    private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Kind kind;
    private final int size;

    UiIcon(Kind kind, int color, int size) {
        this.kind = kind; this.size = size;
        paint.setColor(color);
        paint.setStyle(Paint.Style.STROKE);
        paint.setStrokeWidth(1.8f);
        paint.setStrokeCap(Paint.Cap.ROUND);
        paint.setStrokeJoin(Paint.Join.ROUND);
    }

    @Override public void draw(Canvas canvas) {
        int saved = canvas.save();
        canvas.translate(getBounds().left, getBounds().top);
        canvas.scale(getBounds().width() / 24f, getBounds().height() / 24f);
        switch (kind) {
            case WIFI -> {
                path(canvas, new float[]{3, 8, 12, 1, 21, 8}, true);
                path(canvas, new float[]{6, 12, 12, 7, 18, 12}, true);
                path(canvas, new float[]{9, 16, 12, 13.5f, 15, 16}, true);
                canvas.drawCircle(12, 19, .8f, paint);
            }
            case REFRESH -> {
                canvas.drawArc(new RectF(4, 4, 20, 20), 42, 286, false, paint);
                path(canvas, new float[]{20, 4, 20, 9, 15, 9}, false);
            }
            case CHECK -> {
                Path shield = new Path();
                shield.moveTo(12, 2.8f); shield.lineTo(20, 6); shield.lineTo(19.2f, 13);
                shield.quadTo(18, 18, 12, 21.2f); shield.quadTo(6, 18, 4.8f, 13);
                shield.lineTo(4, 6); shield.close(); canvas.drawPath(shield, paint);
                path(canvas, new float[]{8, 11.8f, 10.8f, 14.6f, 16.2f, 9.2f}, false);
            }
            case USER -> {
                canvas.drawCircle(12, 7.3f, 3.7f, paint);
                Path shoulders = new Path(); shoulders.moveTo(4.5f, 20);
                shoulders.cubicTo(4.5f, 11.2f, 19.5f, 11.2f, 19.5f, 20);
                canvas.drawPath(shoulders, paint);
            }
            case CHEVRON -> path(canvas, new float[]{8, 10, 12, 14, 16, 10}, false);
            case EYE, EYE_OFF -> {
                Path eye = new Path(); eye.moveTo(2.5f, 12);
                eye.quadTo(12, .8f, 21.5f, 12); eye.quadTo(12, 23.2f, 2.5f, 12);
                canvas.drawPath(eye, paint); canvas.drawCircle(12, 12, 2.8f, paint);
                if (kind == Kind.EYE_OFF) canvas.drawLine(4, 3.5f, 20, 20.5f, paint);
            }
            case LOGOUT -> {
                path(canvas, new float[]{10, 4, 5, 4, 5, 20, 10, 20}, false);
                canvas.drawLine(10, 12, 21, 12, paint);
                path(canvas, new float[]{17, 8, 21, 12, 17, 16}, false);
            }
            case HISTORY -> {
                canvas.drawCircle(12, 12, 8.3f, paint);
                path(canvas, new float[]{12, 7, 12, 12, 15.5f, 14}, false);
            }
            case GLOBE -> {
                canvas.drawCircle(12, 12, 8.5f, paint);
                canvas.drawOval(new RectF(8, 3.5f, 16, 20.5f), paint);
                canvas.drawLine(4, 8, 20, 8, paint);
                canvas.drawLine(4, 16, 20, 16, paint);
            }
            case PLUS -> {
                canvas.drawLine(5, 12, 19, 12, paint);
                canvas.drawLine(12, 5, 12, 19, paint);
            }
            case MORE -> {
                canvas.drawCircle(12, 5, .8f, paint);
                canvas.drawCircle(12, 12, .8f, paint);
                canvas.drawCircle(12, 19, .8f, paint);
            }
            case OPEN -> {
                path(canvas, new float[]{13, 4, 20, 4, 20, 11}, false);
                canvas.drawLine(11, 13, 20, 4, paint);
                path(canvas, new float[]{9, 4, 4, 4, 4, 20, 20, 20, 20, 15}, false);
            }
            case HELP -> {
                canvas.drawCircle(12, 12, 8.4f, paint);
                Path question = new Path(); question.moveTo(9.3f, 9);
                question.cubicTo(9.5f, 5.5f, 16.4f, 6, 14.9f, 10);
                question.quadTo(14.4f, 11, 12.4f, 12.3f); question.lineTo(12.4f, 13.8f);
                canvas.drawPath(question, paint); canvas.drawCircle(12.4f, 17.1f, .35f, paint);
            }
        }
        canvas.restoreToCount(saved);
    }

    private void path(Canvas canvas, float[] points, boolean quadratic) {
        Path path = new Path(); path.moveTo(points[0], points[1]);
        if (quadratic) path.quadTo(points[2], points[3], points[4], points[5]);
        else for (int i = 2; i < points.length; i += 2) path.lineTo(points[i], points[i + 1]);
        canvas.drawPath(path, paint);
    }
    @Override public int getIntrinsicWidth() { return size; }
    @Override public int getIntrinsicHeight() { return size; }
    @Override public void setAlpha(int alpha) { paint.setAlpha(alpha); invalidateSelf(); }
    @Override public void setColorFilter(ColorFilter filter) { paint.setColorFilter(filter); invalidateSelf(); }
    @Override public int getOpacity() { return PixelFormat.TRANSLUCENT; }
}
