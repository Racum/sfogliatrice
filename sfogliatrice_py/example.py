import json
import sfogliatrice

# Input can be a dict (already-parsed GeoJSON) or a JSON string.
geojson = {
    "type": "Polygon",
    "coordinates": [[
        [-15.332574, 28.217488],
        [-15.865546, 28.217488],
        [-15.865546, 27.719770],
        [-15.332574, 27.719770],
        [-15.332574, 28.217488],
    ]],
}

# Default parameters — 5 km strips over Gran Canaria.
result = sfogliatrice.tessellate(geojson)

print(f"Targets:       {len(result['targets']['features'])} features")
print(f"Coverages:     {len(result['coverages']['features'])} features")
print(f"Intermediates: {len(result['intermediates']['features'])} features")

# Custom parameters.
result = sfogliatrice.tessellate(
    geojson,
    strip_width=10_000,
    max_strip_length=80_000,
    min_overlap=500,
    heading=45,
)

print(f"\nWith 10 km strips at 45°:")
print(f"Targets:       {len(result['targets']['features'])} features")

# Each field is a standard GeoJSON FeatureCollection dict — pass straight to
# any GeoJSON-aware library (shapely, geopandas, geojson, folium, etc.).
first_target = result["targets"]["features"][0]
print(f"\nFirst target geometry type: {first_target['geometry']['type']}")
print(json.dumps(first_target, indent=2))
