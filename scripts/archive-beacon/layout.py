"""Shared layout constants for "The Archive Beacon".

All scene coordinates are pixels on the 3840x1920 equirectangular panorama.
Yaw 0 / pitch 0 is the panorama centre; 1 degree is PANO_W / 360 pixels and
positive pitch looks down (see src/camera.c).
"""
PANO_W, PANO_H = 3840, 1920
PX_PER_DEG = PANO_W / 360.0
SEED = 20260925
FPS = 30
DURATION = 30

# Station hub and its static collision ring.
CX, CY = 1920, 960
RING_R = 230

# Solar panels (dynamic boxes held by two springs each).
PANEL_W, PANEL_H = 300, 110
PANEL_L = (1440, 960)
PANEL_R = (2400, 960)
ANCHOR_DY = -260

# Procedural planet: 3D sphere widened to offset equirectangular squeeze.
PLANET_X, PLANET_Y = 1330, 1470
PLANET_R = 280
PLANET_SX = 1.45
PLANET_RX, PLANET_RY = PLANET_R * PLANET_SX, PLANET_R

# Rectangle kept free of 2D stars so 3D station parts stay visible.
STATION_CLEAR = (1580, 790, 2260, 1100)


def yaw(x):
    return (x - PANO_W / 2) / PX_PER_DEG


def pitch(y):
    return (y - PANO_H / 2) / PX_PER_DEG


def y_at_pitch(p):
    return PANO_H / 2 + p * PX_PER_DEG


def x_at_yaw(yv):
    return PANO_W / 2 + yv * PX_PER_DEG
