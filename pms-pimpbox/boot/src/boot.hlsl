// PIMPBOX's boot animation, drawn by the graphics card: every pixel of it, from arithmetic. What moves is worked out in
// lib.rs, once a frame, and handed over below; this only says what color each pixel is.
// The numbers that set the pace and the dots that make the marks are written above this by `gpu::header`, from the ones there.

cbuffer Moment : register(b0) {
    float4 u_clock;   // the time, the flash, how dim, and whether it's the short one (which the mark only comes up in, unshaken)
    float4 u_cam0;    // the roll's sine and cosine, the zoom, how far round it's swung
    float4 u_cam1;    // where it is across and along, and where the ball is from it
    float4 u_cam2;    // that swing's sine and cosine, how many dice, how many knocks
    float4 u_cam3;    // how far it's shaken, across and down; and half how wide the picture is (it's H tall, and as wide as it's shown)
    float4 u_ball0;   // where, how big, how solid (not at all: there isn't one)
    float4 u_ball1;   // how wide and tall, how far turned, how far melted
    float4 u_ball2;   // where in the hall, how high, how far off
    float4 u_thrown0; // where, how wide and tall
    float4 u_thrown1; // how far rolled
    float4 u_fish0;   // where, half its length, where the floor is
    float4 u_fish1;   // its heading, and which side of the ball: 0 nowhere, 1 behind, 2 in front
    float4 u_lands[5];
    float4 u_knocks[8];
    float4 u_dice[144]; // where, half its width, how solid; then its two turns, whether it's behind the ball, and how far off it is
};

Texture2D<float> MarkFar : register(t0);
Texture2D<float> WordFar : register(t1);
Texture2D<float> Maker : register(t2);
Texture2D<float> Small : register(t3);

#define T u_clock.x
static const float PI = 3.14159265;
static const float TAU = 6.2831853;
static const float CX = W / 2.0;
static const float CY = H / 2.0;
static const float3 GOLD = float3(1.0, 0.76, 0.2);
static const float3 HOT = float3(1.0, 0.93, 0.7);
static const float3 PURPLE = float3(0.69, 0.45, 1.0);
static const float3 DEEP = float3(0.3, 0.14, 0.5);
static const float3 PINK = float3(1.0, 0.55, 0.8);
static const float3 LIME = float3(0.62, 1.0, 0.6);
static const float3 DICE_BODY = float3(0.48, 0.24, 0.8);

float sq(float x) { return x * x; }
float ease(float a, float b, float x) {
    float t = saturate((x - a) / (b - a));
    return t * t * (3.0 - 2.0 * t);
}
void lay(inout float3 under, float3 over, float alpha) { under = lerp(under, over, saturate(alpha)); }
float reme(float a, float b) { return a - b * floor(a / b); }
int remi(int a, int b) {
    int r = a % b;
    return r < 0 ? r + b : r;
}
float hash(int a, int b) {
    uint n = ((uint)a * 374761393u ^ (uint)b * 668265263u) * 1274126177u;
    return (float)((n ^ (n >> 15)) & 0xFFFFu) / 65535.0;
}

float mark_far(float x, float y) {
    float fx = (x + ROOM) * FINE - 0.5, fy = (y + ROOM) * FINE - 0.5;
    float inx = clamp(fx, 0.0, MARK_W - 1.0 - 0.001), iny = clamp(fy, 0.0, MARK_H - 1.0 - 0.001);
    float outside = length(float2(fx - inx, fy - iny)) / FINE;
    int x0 = (int)inx, y0 = (int)iny;
    float tx = inx - x0, ty = iny - y0;
    float a = MarkFar.Load(int3(x0, y0, 0)), b = MarkFar.Load(int3(x0 + 1, y0, 0));
    float c = MarkFar.Load(int3(x0, y0 + 1, 0)), d = MarkFar.Load(int3(x0 + 1, y0 + 1, 0));
    return lerp(lerp(a, b, tx), lerp(c, d, tx), ty) + outside;
}

float word_far(float x, float y) {
    float fx = x + WORD_ROOM, fy = y + WORD_ROOM;
    if (fx < 0.0 || fy < 0.0 || fx >= WORD_W - 1.0 || fy >= WORD_H - 1.0) {
        return 1e3;
    }
    int x0 = (int)fx, y0 = (int)fy;
    float tx = fx - x0, ty = fy - y0;
    float a = WordFar.Load(int3(x0, y0, 0)), b = WordFar.Load(int3(x0 + 1, y0, 0));
    float c = WordFar.Load(int3(x0, y0 + 1, 0)), d = WordFar.Load(int3(x0 + 1, y0 + 1, 0));
    return lerp(lerp(a, b, tx), lerp(c, d, tx), ty);
}

// How far the floor's lifted at a place in the hall: a wave going out from wherever the ball's come down, at the pace the rings
// of light go out at, dying away as it goes.
float wave(float2 at) {
    float h = 0.0;
    [loop] for (int i = 0; i < 5; i++) {
        float s = T - LANDS[i];
        if (s >= 0.0) {
            float from = length(at - u_lands[i].xy) - s * 3.4;
            h += 0.08 * min(s * 12.0, 1.0) * exp(-s * 1.6) * exp(-sq(from / 0.8)) * cos(from * 5.0);
        }
    }
    return h;
}

// What's to be seen from a place in the hall (across it, off its floor, along it) looking one way: the great round dark it's
// in, a long way off, and its floor or its ceiling over that. `rim` is how bright the dark is; `rippled`, whether the floor and
// ceiling are to be seen moving as the ball's landings shake them, or flat.
float3 surroundings(float3 from, float3 dir, float rim, bool rippled) {
    float3 px = 0.0;
    float up = asin(-dir.y / length(dir));
    float around = atan2(dir.x, dir.z);
    const float SQUARES = 28.0;
    float su = reme(around / PI * SQUARES, 1.0), sv = reme(up / PI * SQUARES, 1.0);
    float sedge = min(min(su, 1.0 - su), min(sv, 1.0 - sv));
    float stone = abs(abs(su - 0.5) + abs(sv - 0.5) - 0.3);
    float ruled = exp(-sq(sedge / 0.03)) + 0.6 * exp(-sq(stone / 0.026));
    float struck = 0.0;
    [loop] for (int l = 0; l < 5; l++) {
        float s = T - LANDS[l];
        if (s >= 0.0 && T < HIT) {
            struck += (1.0 - exp(-s * 40.0)) * exp(-s * 6.0);
        }
    }
    // And never quite still: slow swells of light going round it and up and down it, across each other.
    float alive = 1.0 + 0.2 * sin(around * 3.0 + T * 1.3) * sin(up * 4.0 - T * 0.9) + 0.1 * sin(around * 5.0 - up * 3.0 - T * 2.1);
    px += float3(0.2, 0.08, 0.4) * rim * (0.4 * ruled + 0.05) * (1.0 + 0.9 * struck) * alive;
    float steep = abs(dir.y);
    if (steep < 1e-4) {
        return px;
    }
    bool overhead = dir.y > 0.0;
    float room = overhead ? 1.0 + CEILING - from.y : from.y;
    float away = room / steep;
    float2 at = from.xz + dir.xz * away;
    float lifted = 0.0;
    if (rippled) {
        for (int n = 0; n < 2; n++) {
            lifted = wave(at) * (overhead ? 0.6 : 1.0);
            away = (room - lifted) / steep;
            at = from.xz + dir.xz * away;
        }
    }
    float lit = 0.0, rings = 0.0;
    int i;
    [loop] for (i = 0; i < 5; i++) {
        if (T >= LANDS[i]) {
            float gone = length(at - u_lands[i].xy);
            float reach = (T - LANDS[i]) * 3.4;
            lit += ease(reach, reach - 1.5, gone) * 0.3;
            for (int k = 0; k < 3; k++) {
                float r = reach - k * 0.8;
                if (r > 0.0) {
                    rings += exp(-sq((gone - r) / 0.16)) / (1.0 + r * 0.4);
                }
            }
        }
    }
    int knocks = (int)u_cam2.w;
    [loop] for (i = 0; i < knocks; i++) {
        float4 k = u_knocks[i];
        float gone = length(at - k.xy);
        rings += k.w * exp(-sq((gone - k.z * 2.4) / 0.11)) * exp(-k.z * 3.5);
    }
    float under = length(at - u_ball2.xy);
    lit += u_ball0.w * 0.9 * exp(-under * under * 0.5) / (0.6 + u_ball2.z);
    const float TILE = 0.9;
    float u = reme(at.x / TILE, 1.0), v = reme(at.y / TILE, 1.0);
    float edge = min(min(u, 1.0 - u), min(v, 1.0 - v)) * TILE;
    float thin = 0.005 * away + 0.003;
    float lines = exp(-sq(edge / thin)) + 0.22 * exp(-sq(edge / (thin * 6.0)));
    v = overhead ? v : 1.0 - v;
    float col = floor(u * (COLS + 5.0) - 2.5), row = floor(v * (ROWS + 5.0) - 2.5);
    float sign_on = 0.0;
    if (col >= 0.0 && col < COLS && row >= 0.0 && row < ROWS) {
        sign_on = (float)((DOLLAR[(int)row] >> (10 - (int)col)) & 1u);
    }
    // Brighter where it's lifted, a little: the crest of the wave catches the light.
    float reach = ease(12.0, 6.0, away) / (1.0 + 0.05 * away * away) * (1.0 + 4.0 * max(lifted, 0.0));
    float3 color = overhead ? PURPLE : GOLD;
    px += color * reach * (lines * (0.4 + 0.9 * lit + rings) + sign_on * (0.1 + 0.75 * lit + 0.7 * rings));
    px += HOT * reach * rings * 0.2 * min(lines + sign_on, 1.0);
    return px;
}

// A way of looking, as the camera has it (across, up, ahead), as the hall has it: turned by as far as the camera's swung round.
float3 in_hall(float3 d) { return float3(d.x * u_cam2.y - d.z * u_cam2.x, d.y, d.x * u_cam2.x + d.z * u_cam2.y); }

// Where the camera is in the hall: as far from the ball as it keeps, round it by as far as it's swung.
float3 eye_at() {
    float3 back = in_hall(float3(-u_cam1.z, 0.0, -u_cam1.w));
    return float3(u_cam1.x + u_cam1.z + back.x, 1.0, u_cam1.y + u_cam1.w + back.z);
}

void hall(float x, float y, inout float3 px) {
    if (T < BALL || T >= HIT) {
        return;
    }
    // Where the camera is in the hall: as far from the ball as it keeps, round it by as far as it's swung.
    float3 eye = eye_at();
    float rim = 0.3 + 0.7 * min(length(float2(x - CX, y - CY)) / 400.0, 1.0);
    px += surroundings(eye, in_hall(float3((x - CX) / GROUND, (CY - y) / GROUND, 1.0)), rim, true);
}

float3 dolphin_round(float from, float thick) {
    float e = from / thick;
    float toward = sqrt(max(1.0 - e * e, 0.0));
    float3 color = float3(0.45, 0.42, 0.62);
    lay(color, float3(0.9, 0.88, 0.97), ease(-0.1, -0.5, e));
    float lit = clamp(0.25 + 0.5 * e + 0.55 * toward, 0.0, 1.2);
    return color * (0.28 + 0.8 * lit) + pow(max(0.62 * e + 0.78 * toward, 0.0), 14.0) * 0.4;
}

bool dolphin_at(float along, float above, float wag, out float3 color) {
    const float3 FIN = float3(0.3, 0.27, 0.46);
    color = 0.0;
    if (along < -1.1 || along >= 1.04 || abs(above) > 0.7) {
        return false;
    }
    float tail = max(-along, 0.0);
    float from = above - (-0.16 * along * along + 0.2 * wag * tail * tail);
    float thick;
    if (along > 0.2) {
        float u = (along - 0.2) / 0.66;
        thick = u < 1.0 ? 0.2 * pow(1.0 - u * u, 0.7) : 0.0;
    } else {
        float u = (0.2 - along) / 1.12;
        thick = max(0.2 * (1.0 - u * u), along > -0.95 ? 0.035 : 0.0);
    }
    if (abs(from) < thick) {
        if (length(float2(along - 0.7, from - 0.03)) < 0.02) {
            color = float3(0.03, 0.02, 0.06);
        } else if (along >= 0.5 && along < 0.555) {
            color = GOLD * (0.45 + 0.7 * sqrt(max(1.0 - sq(from / thick), 0.0)));
        } else {
            color = dolphin_round(from, thick);
        }
        return true;
    }
    if (along > 0.76) {
        float u = (along - 0.76) / 0.26;
        float beak = 0.045 * sqrt(max(1.0 - u * u, 0.0));
        if (abs(from + 0.04) < beak) {
            color = dolphin_round(from + 0.04, beak);
            return true;
        }
    }
    float up = (from - thick + 0.02) / 0.22;
    if (up >= 0.0 && up < 1.0 && along < 0.22 - 0.3 * up && along > -0.16 + 0.05 * up + 0.05 * sin(PI * up)) {
        color = FIN * (1.0 + 0.5 * up);
        return true;
    }
    float under = (-from - thick + 0.03) / 0.2;
    if (under >= 0.0 && under < 1.0 && along < 0.46 - 0.3 * under && along > 0.26 - 0.16 * under + 0.04 * sin(PI * under)) {
        color = FIN * 0.85;
        return true;
    }
    float outward = (-0.8 - along) / 0.22;
    if (outward > 0.0 && abs(from) < 0.2 * pow(min(outward, 1.0), 0.8) && along > -1.06 + 0.1 * max(1.0 - abs(from) / 0.2, 0.0)) {
        color = FIN * (1.0 + 0.3 * (from / 0.2));
        return true;
    }
    return false;
}

void dolphin(float x, float y, float which, inout float3 px) {
    if (u_fish1.z != which) {
        return;
    }
    float reach = u_fish0.z;
    float dx = x - u_fish0.x, dy = y - u_fish0.y;
    if (abs(dx) > reach * 1.15 || abs(dy) > reach * 1.15 || y > u_fish0.w) {
        return;
    }
    float2 heading = u_fish1.xy;
    float up = heading.x < 0.0 ? 1.0 : -1.0;
    float wag = sin(T * 10.0);
    float3 all = 0.0;
    float cover = 0.0;
    static const float2 SPOTS[4] = { float2(-0.25, -0.25), float2(0.25, -0.25), float2(-0.25, 0.25), float2(0.25, 0.25) };
    for (int i = 0; i < 4; i++) {
        float ex = dx + SPOTS[i].x, ey = dy + SPOTS[i].y;
        float3 color;
        if (dolphin_at((ex * heading.x + ey * heading.y) / reach, (ey * heading.x - ex * heading.y) * up / reach, wag, color)) {
            all += color * 0.25;
            cover += 0.25;
        }
    }
    if (cover > 0.0) {
        lay(px, all / cover, cover);
    }
}

void goo(float x, float y, inout float3 px) {
    if (T < GOO || T >= BALL) {
        return;
    }
    float since = T - GOO;
    static const float4 LUMPS_A[8] = {
        float4(90.0, 52.0, 26.0, 2.0), float4(300.0, 96.0, -17.0, -1.5), float4(520.0, 40.0, 21.0, 3.0), float4(610.0, 128.0, -12.0, 1.0),
        float4(60.0, 418.0, 19.0, -2.0), float4(250.0, 448.0, -24.0, 1.5), float4(450.0, 396.0, 14.0, 2.5), float4(580.0, 440.0, -19.0, -1.0),
    };
    static const float2 LUMPS_B[8] = {
        float2(13.0, 0.9), float2(9.0, -1.3), float2(11.0, 0.6), float2(7.0, 1.7), float2(10.0, -0.8), float2(14.0, 1.1), float2(8.0, -1.9), float2(12.0, 0.5),
    };
    const float EDGE = 30.0;
    for (int i = 0; i < 8; i++) {
        float4 a = LUMPS_A[i];
        float size = LUMPS_B[i].x;
        float lx = reme(a.x + a.z * since + EDGE, W + 2.0 * EDGE) - EDGE, ly = a.y + a.w * since;
        float dx = x - lx, dy = y - ly;
        float from = length(float2(dx, dy));
        if (from > size * 1.5) {
            continue;
        }
        float around = atan2(dy, dx) - LUMPS_B[i].y * since + i * 2.0;
        float edge = size * (1.0 + 0.16 * sin(3.0 * around) + 0.09 * sin(5.0 * around + 1.3));
        px += LIME * (0.35 + size / 28.0) * exp(-sq((from - edge) / 0.9));
    }
    float r = BALL_R * GROUND / AHEAD;
    float bx = u_thrown0.x, by = u_thrown0.y, wide = u_thrown0.z, tall = u_thrown0.w;
    float dx = x - bx, dy = y - by;
    if (abs(dx) > r * 2.0 || abs(dy) > r * 2.0) {
        return;
    }
    float outward = length(float2(dx / (r * wide), dy / (r * tall)));
    float from_edge = (outward - 1.0) * r * min(wide, tall);
    px += LIME * exp(-sq(from_edge / 2.0));
    px += LIME * 0.1 * ease(1.15, 0.9, outward);
    float mx = bx + cos(u_thrown1.x) * r * 0.55 * wide, my = by + sin(u_thrown1.x) * r * 0.55 * tall;
    px += LIME * 0.9 * exp(-sq((length(float2(x - mx, y - my)) - 5.0) / 1.4));
}

// The mirror ball: rows of small flat mirrors, each facing its own way and showing what's that way of it in the hall (the
// floor, the ceiling, the rings going out under it), tinted gold or purple; and flashing, as it turns, when it throws one of
// the hall's lights straight at us.
void mirror_ball(float x, float y, inout float3 px) {
    float alpha = u_ball0.w;
    if (alpha <= 0.0) {
        return;
    }
    float r = u_ball0.z;
    float u = (x - u_ball0.x) / (r * u_ball1.x), v = (y - u_ball0.y) / (r * u_ball1.y);
    float r2 = u * u + v * v;
    if (r2 >= 1.0) {
        px += GOLD * alpha * 0.18 * exp(-(sqrt(r2) - 1.0) * 3.5);
        return;
    }
    float nz = sqrt(1.0 - r2);
    // Where on it this is, in the hall's terms, and then in its own, turned as it is.
    float lat = asin(v), hall_lon = atan2(u, nz) + u_cam0.w, lon = hall_lon + u_ball1.z;
    const float BANDS = 13.0;
    float band = (lat / PI + 0.5) * BANDS;
    float bi = floor(band);
    // Each row has as many mirrors as fit round it at its own height, and starts a little round from the next.
    float mid = ((bi + 0.5) / BANDS - 0.5) * PI;
    float facets = max(round(2.0 * BANDS * cos(mid)), 5.0);
    float set = hash((int)bi, 7);
    float facet = lon / TAU * facets + set;
    float fi = floor(facet);
    float bf = band - bi, ff = facet - fi;
    float grout = ease(0.0, 0.07, min(bf, 1.0 - bf)) * ease(0.0, 0.09, min(ff, 1.0 - ff));
    float kind = hash((int)bi, (int)reme(fi, facets));
    // Which way the mirror faces, flat as it is; and the ball's own roundness there, which it has a little of.
    float flon = (fi + 0.5 - set) / facets * TAU - u_ball1.z;
    float3 flat = float3(sin(flon) * cos(mid), -sin(mid), -cos(flon) * cos(mid));
    float3 curved = float3(sin(hall_lon) * cos(lat), -v, -cos(hall_lon) * cos(lat));
    // From us to it, and off it.
    float3 toward = in_hall(normalize(float3(u_cam1.z, u_ball2.z - 1.0, u_ball2.w)));
    float3 off_flat = reflect(toward, flat);
    float3 off = reflect(toward, normalize(lerp(flat, curved, 0.3)));
    float3 seen = surroundings(float3(u_ball2.x, u_ball2.z, u_ball2.y), off, 1.0, false);
    float3 tint = kind < 0.68 ? float3(1.0, 0.8, 0.36) : (kind < 0.9 ? float3(0.74, 0.54, 1.0) : float3(1.0, 0.95, 0.86));
    float light = max(dot(flat, normalize(float3(-0.45, 0.6, -0.66))), 0.0) + 0.15 * max(flat.y, 0.0);
    float3 face = tint * (0.13 + 0.5 * light) + tint * seen * 1.7;
    // The hall's lights, thrown at us by whichever mirrors happen to be turned just so.
    face += float3(1.0, 1.0, 1.0) * 2.4 * pow(max(dot(off_flat, normalize(float3(-0.5, 0.7, -0.5))), 0.0), 70.0);
    face += PINK * 2.0 * pow(max(dot(off_flat, normalize(float3(0.7, 0.45, 0.55))), 0.0), 70.0);
    face += HOT * 2.0 * pow(max(dot(off_flat, normalize(float3(0.1, 0.9, -0.42))), 0.0), 70.0);
    face += PURPLE * 1.8 * pow(max(dot(off_flat, normalize(float3(-0.8, 0.2, 0.56))), 0.0), 70.0);
    // The row round its middle has the mark printed on every mirror, in white: they're mirrors still, round it.
    if (bi == floor(BANDS / 2.0)) {
        float row = floor((bf - 0.12) / 0.76 * ROWS), col = floor((ff - 0.5) / (0.76 * COLS / ROWS) * COLS + COLS / 2.0);
        if (row >= 0.0 && row < ROWS && col >= 0.0 && col < COLS && ((DOLLAR[(int)row] >> (10 - (int)col)) & 1u) == 1u) {
            // Not at first: they come up toward the end of its time in the hall.
            lay(face, float3(1.0, 1.0, 1.0), 0.92 * saturate((T - (LAUNCH - 1.0)) / 0.8));
        }
    }
    face *= lerp(0.16, 1.0, grout) * (0.7 + 0.3 * nz);
    float3 gold = GOLD + HOT * ease(0.72, 0.95, r2) * 0.35;
    lay(face, gold, u_ball1.w);
    lay(px, face, alpha * saturate((1.0 - r2) * r * 0.6));
}

float3 turned(float3 v, float2 a, float2 b) {
    float y = v.y * a.y - v.z * a.x, z = v.y * a.x + v.z * a.y;
    return float3(v.x * b.y + z * b.x, y, -v.x * b.x + z * b.y);
}

bool pip_at(float p, float q, float a, float b) { return length(float2(p - a * 0.5, q - b * 0.5)) < 0.19; }

// `turned`, undone.
float3 unturned(float3 v, float2 a, float2 b) {
    float x = v.x * b.y - v.z * b.x, z = v.x * b.x + v.z * b.y;
    return float3(x, v.y * a.y + z * a.x, -v.y * a.x + z * a.y);
}

// Whether there's a pip of the face numbered `number` at (`p`, `q`) on it.
bool pips(int number, float p, float q) {
    bool middle = pip_at(p, q, 0.0, 0.0);
    bool corners = pip_at(p, q, -1.0, -1.0) || pip_at(p, q, 1.0, 1.0);
    bool others = pip_at(p, q, -1.0, 1.0) || pip_at(p, q, 1.0, -1.0);
    if (number == 1) return middle;
    if (number == 2) return corners;
    if (number == 3) return corners || middle;
    if (number == 4) return corners || others;
    if (number == 5) return corners || others || middle;
    return corners || others || pip_at(p, q, -1.0, 0.0) || pip_at(p, q, 1.0, 0.0);
}

// A die, where the line of sight `u` across and `v` down from its middle (in half-widths) meets it, turned by `turn`: a block
// of purple glass with its edges and corners rounded off, smoky in the middle of each face inside a paler frame, with gold
// pips. What's behind it is seen through it, bent, and most where it rounds; so are its own far sides, and their pips, faintly;
// and its rounded edges catch the light in more colors than one. (`x`, `y`) is where in the picture this is.
bool die_at(float u, float v, float2 turn, float x, float y, float away, out float3 color) {
    color = 0.0;
    if (u * u + v * v > 3.0) {
        return false;
    }
    float2 a = float2(sin(turn.x), cos(turn.x)), b = float2(sin(turn.y), cos(turn.y));
    float3 from = turned(float3(u, v, -4.0), a, b), way = turned(float3(0.0, 0.0, 1.0), a, b), light = normalize(turned(float3(-0.45, -0.6, -0.66), a, b));
    // The square block it's cut from: where the line goes into that, and where it comes out.
    float nearest = -1e30, farthest = 1e30;
    int out_face = 0;
    for (int i = 0; i < 3; i++) {
        float w = abs(way[i]) < 1e-6 ? 1e-6 : way[i];
        float t1 = (-1.0 - from[i]) / w, t2 = (1.0 - from[i]) / w;
        nearest = max(nearest, min(t1, t2));
        if (max(t1, t2) < farthest) {
            farthest = max(t1, t2);
            out_face = i;
        }
    }
    if (nearest >= farthest) {
        return false;
    }
    // And from there, step by step, to the die itself, which is that with its edges rounded: or past it, at a corner.
    const float ROUND = 0.2;
    float gone = nearest;
    float3 hit = from + way * gone;
    bool met = false;
    [loop] for (int s = 0; s < 32; s++) {
        hit = from + way * gone;
        float3 q = abs(hit) - (1.0 - ROUND);
        float off = length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0) - ROUND;
        if (off < 0.001) {
            met = true;
            break;
        }
        gone += off;
        if (gone > farthest) {
            break;
        }
    }
    if (!met) {
        return false;
    }
    // Which way it faces there, as the die has it and as we do (up being up); and which face that's on, or nearest.
    float3 faces = sign(hit) * normalize(max(abs(hit) - (1.0 - ROUND), 0.0) + 1e-5);
    float3 flat = abs(faces);
    int face = flat.x >= flat.y && flat.x >= flat.z ? 0 : (flat.y >= flat.z ? 1 : 2);
    float side = faces[face] > 0.0 ? 1.0 : -1.0;
    float rounds = 1.0 - flat[face];
    float3 leaves = from + way * farthest;
    float out_side = way[out_face] > 0.0 ? 1.0 : -1.0;
    float p = hit[(face + 1) % 3], q = hit[(face + 2) % 3];
    float p2 = leaves[(out_face + 1) % 3], q2 = leaves[(out_face + 2) % 3];
    float3 ours = unturned(faces, a, b);
    ours.y = -ours.y;
    // The hall behind it, bent on the way through: the more glass there is to go through, the more.
    float3 sight = normalize(float3((x - CX) / GROUND, (CY - y) / GROUND, 1.0));
    float through = farthest - gone;
    float3 bent = refract(sight, ours, 0.8);
    bent = normalize(lerp(sight, bent, 0.35 + 0.3 * through));
    float3 seen = surroundings(eye_at(), in_hall(bent), 0.8, true);
    float lit = max(dot(faces, light), 0.0);
    float glancing = pow(1.0 - abs(dot(faces, way)), 3.0);
    // Smoky in the middle of a face, darker toward one corner of it than the other; clear in the frame round that.
    float out_to = max(abs(p), abs(q));
    float smoke = lerp(0.5 + 0.12 * (p + q), 1.0, ease(0.6, 0.74, out_to));
    // Where it rounds it's frosted: less of what's behind it comes through, and that's as well, bent as it is there.
    float frost = ease(0.02, 0.3, rounds);
    color = seen * float3(0.8, 0.62, 1.0) * 1.15 * smoke * (1.0 - 0.7 * frost) + DICE_BODY * (0.14 + 0.06 * through + 0.22 * lit + 0.2 * frost);
    // Its far sides: their edges, and their pips.
    color += PURPLE * 0.3 * ease(0.8, 1.0, max(abs(p2), abs(q2)));
    if (pips(out_side > 0.0 ? out_face + 1 : 6 - out_face, p2, q2)) {
        color = lerp(color, GOLD * 0.7, 0.4);
    }
    // Its near side: a pip, or glass; the inner edge of its frame, a thin pale line.
    if (rounds < 0.02 && pips(side > 0.0 ? face + 1 : 6 - face, p, q)) {
        color = GOLD * (0.45 + 0.8 * lit);
    }
    color += float3(0.8, 0.7, 1.0) * 0.3 * exp(-sq((out_to - 0.74) / 0.022)) * step(rounds, 0.02);
    // Where it rounds, it catches the light, and splits it: a different color each way it faces.
    float3 split = 0.5 + 0.5 * cos(TAU * (dot(ours, float3(0.5, 0.35, 0.2)) + 0.5 * rounds + float3(0.0, 0.33, 0.67)));
    color += lerp(float3(0.82, 0.7, 1.0), split, 0.5) * 0.6 * ease(0.02, 0.4, rounds);
    color += float3(0.82, 0.7, 1.0) * 0.4 * glancing;
    color += float3(1.0, 0.95, 1.0) * 0.8 * pow(max(dot(reflect(way, faces), light), 0.0), 24.0);
    // Two lamps where the ball is: an orange one down on the floor under it, and a purple one up on the ceiling over it.
    // Each lights the die from where it is, less the further off; and for a moment, each time the ball comes down, a lot more.
    if (u_ball0.w > 0.0) {
        float struck = 0.0;
        [loop] for (int l = 0; l < 5; l++) {
            float s = T - LANDS[l];
            if (s >= 0.0 && T < LAUNCH) {
                struck += exp(-s * 9.0);
            }
        }
        // As we see them: across, down, and away from us.
        float3 here = float3((x - CX) * away / GROUND, (y - CY) * away / GROUND, away);
        float across = (u_ball0.x - CX) * u_ball2.w / GROUND;
        for (int k = 0; k < 2; k++) {
            float3 to = float3(across, k == 0 ? 1.0 : -CEILING, u_ball2.w) - here;
            float3 toward = turned(normalize(to), a, b);
            float reaches = (0.9 + 3.2 * struck) / (1.0 + 0.12 * dot(to, to));
            float3 lamp = k == 0 ? float3(1.0, 0.5, 0.12) : float3(0.62, 0.36, 1.0);
            color += lamp * reaches * (0.5 * max(dot(faces, toward), 0.0) + 0.8 * pow(max(dot(reflect(way, faces), toward), 0.0), 16.0));
        }
    }
    return true;
}

void dice(float x, float y, float behind, inout float3 px) {
    int count = (int)u_cam2.z;
    [loop] for (int i = 0; i < count; i++) {
        float4 d = u_dice[i * 2], e = u_dice[i * 2 + 1];
        if (e.z != behind || abs(x - d.x) > d.z * 1.8 || abs(y - d.y) > d.z * 1.8) {
            continue;
        }
        float3 color;
        if (die_at((x - d.x) / d.z, (y - d.y) / d.z, e.xy, x, y, e.w, color)) {
            // Not all of what's straight behind it is hidden.
            lay(px, color, d.w * 0.86);
        }
    }
}

void chain(float x, float y, float cy, bool nearer, inout float3 px) {
    float shown = ease(NAME + 0.9, NAME + 1.5, T);
    if (shown <= 0.0) {
        return;
    }
    const float LINKS = 30.0;
    float wide = 178.0, tall = 50.0;
    float swing = 0.2 * sin(T * 1.7);
    float dx = x - CX, dy = y - (cy + 8.0);
    float ex = (dx * cos(swing) + dy * sin(swing)) / wide, ey = (dy * cos(swing) - dx * sin(swing)) / tall;
    float outward = length(float2(ex, ey));
    if (abs(outward - 1.0) > 0.3 || (ey > 0.0) != nearer) {
        return;
    }
    float around = atan2(ey, ex);
    float across = (outward - 1.0) * (wide * sq(cos(around)) + tall * sq(sin(around)));
    float along = around / TAU * LINKS + T * 1.4;
    float longs = 22.0;
    int link = (int)floor(along);
    float within = (reme(along, 1.0) - 0.5) * longs;
    bool on = remi(link, 2) == 0 ? abs(length(float2(within / (longs * 0.5), across / 6.5)) - 0.82) < 0.2 : (abs(across) < 2.2 && abs(within) < longs * 0.42);
    if (on) {
        float3 gold = GOLD * (nearer ? 1.0 : 0.45) + HOT * (nearer && across < 0.0 ? 0.4 : 0.0);
        lay(px, gold, shown);
    }
}

void power(float x, float y, inout float3 px) {
    if (T >= RAIN || abs(x - CX) > min(T / 0.03, 1.0) * u_cam3.z) {
        return;
    }
    float opens = saturate((T - (RAIN - 3.0 / FPS)) * FPS / 3.0 + 1.0 / 3.0) * step(RAIN - 3.0 / FPS, T);
    float tall = 1.6 + 4.0 * sq(T / RAIN) + 1.2 * abs(sin(T * 95.0)) + 260.0 * opens * opens;
    float bright = (1.1 + 0.35 * sin(T * 140.0) * cos(T * 33.0)) * (1.0 - 0.55 * opens);
    px += LIME * exp(-sq((y - CY) / tall)) * bright;
    px += exp(-sq((y - CY) / (1.1 + 6.0 * opens))) * 0.6;
}

// The name, raining: how lit this pixel is, whether it's on a letter at all, and whether that's the one at the head of its
// column. `fallen` is how many rows down it's all come, and `which` shifts which letters they are: a different rain for each.
float3 rain(float x, float y, int fallen, int which) {
    const float DOT = 2.0;
    int col = (int)floor(x / (6.0 * DOT)), row = (int)floor(y / (8.0 * DOT));
    int head = remi(col * 7 + col * col * 3 + fallen, 37), trail = 7 + remi(col * 5 + which * 3, 9);
    int behind = remi(head - row, 34);
    int u = (int)reme(floor(x / DOT), 6.0), v = (int)reme(floor(y / DOT), 8.0);
    bool on = false;
    if (behind < trail && u >= 0 && u < 5 && v >= 0 && v < 7) {
        on = ((NAME_ROWS[remi(row + col * 3 + which * 2, 7) * 7 + v] >> (4 - u)) & 1u) == 1u;
    }
    return float3(on ? 1.0 - (float)behind / (float)trail : 0.0, on ? 1.0 : 0.0, on && behind == 0 ? 1.0 : 0.0);
}

// How much of a place words cover, the words being lettered twice the size they're shown, with their middle at (CX, `cy`).
float words(Texture2D<float> of, float2 size, float x, float y, float cy) {
    float fx = (x - CX) * 2.0 + size.x / 2.0 - 0.5, fy = (y - cy) * 2.0 + size.y / 2.0 - 0.5;
    if (fx < 0.0 || fy < 0.0 || fx >= size.x - 1.0 || fy >= size.y - 1.0) {
        return 0.0;
    }
    int x0 = (int)fx, y0 = (int)fy;
    float tx = fx - x0, ty = fy - y0;
    float a = of.Load(int3(x0, y0, 0)), b = of.Load(int3(x0 + 1, y0, 0));
    float c = of.Load(int3(x0, y0 + 1, 0)), d = of.Load(int3(x0 + 1, y0 + 1, 0));
    return lerp(lerp(a, b, tx), lerp(c, d, tx), ty);
}

void lockup(float x, float y, inout float3 px) {
    if (T < NAME) {
        return;
    }
    float since = T - NAME;
    const float DOT = 15.0;
    float shake = since < 0.5 && u_clock.w < 0.5 ? 30.0 * exp(-since * 7.0) * cos(since * 40.0) * (1.0 - since / 0.5) : 0.0;
    float cy = 0.41 * H + shake;
    chain(x, y, cy, false, px);
    float from = mark_far((x - CX) / DOT + COLS / 2.0, (y - cy) / DOT + ROWS / 2.0);
    float breath = 1.0 + 0.12 * sin(since * 2.4);
    px += GOLD * breath * 0.55 * exp(-from * 0.55);
    px += float3(1.0, 0.5, 0.1) * breath * 0.3 * exp(-from * 0.22);
    lay(px, HOT, 1.0 - ease(0.0, 0.35, from));

    if (T >= HAT) {
        float s = T - HAT;
        float fall = s < 0.3 ? -sq(1.0 - s / 0.3) * 260.0 : -exp(-(s - 0.3) * 9.0) * abs(sin((s - 0.3) * 22.0)) * 14.0;
        const float BIG = 12.0;
        float top = cy - ROWS / 2.0 * DOT - 6.6 * BIG + fall, left = CX - 8.6 * BIG;
        float row = floor((y - top) / BIG);
        float col = floor((x - left - (8.0 - row) * 2.5) / BIG);
        if (row >= 0.0 && row < 9.0 && col >= 0.0 && col < 17.0) {
            uint code = HAT_DOTS[(int)row * 17 + (int)col];
            if (code == 1u) lay(px, float3(0.48, 0.24, 0.8), 1.0);
            else if (code == 2u) lay(px, DEEP, 1.0);
            else if (code == 3u) lay(px, GOLD, 1.0);
            else if (code == 4u) lay(px, PINK, 1.0);
        }
    }

    chain(x, y, cy, true, px);

    float left = CX - WORD_WIDE / 2.0, top = 0.685 * H + shake;
    float off = word_far(x - left, y - top);
    float fill = 1.0 - ease(WORD_THICK - 1.0, WORD_THICK, off);
    // A band of light goes across it, once, left to right. In the band the letters are only their outlines, lit; a thin
    // upright line of light at each edge of it; and sparks come off its front edge and drift on after it.
    // It takes until the picture starts to go out.
    const float BAND = 44.0, FROM = NAME + 0.5, TAKES = LENGTH - 0.35 - FROM;
    float pace = (WORD_WIDE + 60.0 + BAND) / TAKES;
    float front = left - 30.0 + (T - FROM) * pace;
    // It comes on evenly: nothing until it's a little way into the name, all there by a third of the way across.
    float comes = saturate((front - left - 0.12 * WORD_WIDE) / (0.22 * WORD_WIDE));
    float banded = 0.0;
    if (T >= FROM && T < FROM + TAKES) {
        banded = comes * ease(front - BAND - 2.0, front - BAND + 2.0, x) * ease(front + 2.0, front - 2.0, x);
        float within = comes * (1.0 - ease(0.5, 0.78, abs((y - top) / WORD_TALL - 0.5)));
        px += float3(0.86, 0.74, 1.0) * within * (exp(-sq((x - front) / 1.5)) + 0.55 * exp(-sq((x - front + BAND) / 1.5)));
        px += PURPLE * within * 0.22 * exp(-sq((x - front) / 9.0));
    }
    lay(px, PURPLE, fill * (1.0 - banded));
    // The last letter's outline is green.
    bool last = x > left + WORD_WIDE - 0.86 * WORD_TALL - WORD_THICK - 10.0;
    lay(px, last ? LIME : float3(0.95, 0.88, 1.0), fill * ease(WORD_THICK - 3.4, WORD_THICK - 2.4, off) * banded);
    if (T >= FROM && T < FROM + TAKES + 0.4) {
        [loop] for (int i = 0; i < 40; i++) {
            // Where along the name each comes off, and how far up it: spread out evenly, in no order.
            float born = left + WORD_WIDE * frac(i * 0.618 + 0.07);
            float age = (front - born) / pace;
            if (age > 0.0 && age < 0.5) {
                // On to the right, mostly; a few back to the left.
                float sx = born + age * (45.0 + 80.0 * sin(i * 2.4));
                float sy = top + WORD_TALL * frac(i * 0.381 + 0.27) + age * 46.0 * cos(i * 1.7);
                px += float3(0.92, 0.82, 1.0) * comes * sq(1.0 - age / 0.5) * exp(-(sq(x - sx) + sq(y - sy)) / 1.6);
            }
        }
    }
    if (T >= MADE) {
        lay(px, float3(1.0, 1.0, 1.0), words(Maker, float2(MAKER_W, MAKER_H), x, y, 0.885 * H));
        lay(px, float3(0.8, 0.8, 0.8), words(Small, float2(SMALL_W, SMALL_H), x, y, 0.95 * H));
    }
}

struct Corner {
    float4 place : SV_Position;
    float2 at : TEXCOORD0;
};

// One triangle that covers the picture, whatever size it's shown at: each pixel is told where it is in the picture as drawn.
Corner vs(uint id : SV_VertexID) {
    Corner c;
    float2 uv = float2((id << 1) & 2, id & 2);
    c.place = float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    // As tall as the picture's laid out, and as wide as it's shown: what's in the middle stays in the middle, and there's
    // more to each side on a wide screen.
    c.at = float2(CX + (uv.x - 0.5) * 2.0 * u_cam3.z, uv.y * H);
    return c;
}

float4 ps(Corner c) : SV_Target {
    float dx = (c.at.x - CX - u_cam3.x) / u_cam0.z, dy = (c.at.y - CY - u_cam3.y) / u_cam0.z;
    float x = CX + dx * u_cam0.y - dy * u_cam0.x, y = CY + dx * u_cam0.x + dy * u_cam0.y;
    float3 px = 0.0;
    if (T >= RAIN && T < GOO) {
        float3 r = rain(x, y, (int)floor((T - RAIN) * FPS / 2.0), 0);
        if (T < FLIP - 0.001) {
            px = LIME * r.x + r.z * 0.6;
        } else {
            px = r.y > 0.5 ? float3(0.0, 0.0, 0.0) : LIME;
        }
    }
    power(x, y, px);
    hall(x, y, px);
    goo(x, y, px);
    dolphin(x, y, 1.0, px);
    dice(x, y, 1.0, px);
    mirror_ball(x, y, px);
    dolphin(x, y, 2.0, px);
    dice(x, y, 0.0, px);
    lockup(x, y, px);
    // Each time the ball comes down, the rain again, for as long as one frame at the pace it's meant for: only its letters,
    // over everything, and not the same rain twice.
    [loop] for (int i = 0; i < 5; i++) {
        float s = (T - LANDS[i]) * FPS;
        if (s >= -0.25 && s < 0.75 && T < HIT) {
            float3 r = rain(c.at.x, c.at.y, 5 + i * 9, i + 1);
            lay(px, LIME * (0.35 + 0.65 * r.x) + r.z * 0.6, r.y);
        }
    }
    px += HOT * u_clock.y;
    return float4(saturate(px * u_clock.z), 1.0);
}
