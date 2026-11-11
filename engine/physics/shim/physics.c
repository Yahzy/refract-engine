// A narrow C interface over the physics library: plain arguments and small
// structs only, so that the Rust side never mirrors the library's large
// definition structs (whose layout may change between versions).

#include "box3d/box3d.h"
#include "box3d/collision.h"

#include <stdint.h>

typedef struct rf_material
{
	float density;
	float friction;
	float restitution;
	uint64_t category;
	uint64_t mask;
} rf_material;

// Sizes of the structs passed by value, checked against the Rust mirrors.
void rf_sizes( int32_t* out )
{
	out[0] = (int32_t)sizeof( b3Vec3 );
	out[1] = (int32_t)sizeof( b3Quat );
	out[2] = (int32_t)sizeof( b3Transform );
	out[3] = (int32_t)sizeof( b3WorldId );
	out[4] = (int32_t)sizeof( b3BodyId );
	out[5] = (int32_t)sizeof( rf_material );
	out[6] = (int32_t)sizeof( b3WorldTransform );
}

void rf_set_length_units( float units_per_meter )
{
	b3SetLengthUnitsPerMeter( units_per_meter );
}

b3WorldId rf_create_world( b3Vec3 gravity )
{
	b3WorldDef def = b3DefaultWorldDef();
	def.gravity = gravity;
	return b3CreateWorld( &def );
}

void rf_destroy_world( b3WorldId world )
{
	b3DestroyWorld( world );
}

void rf_step( b3WorldId world, float dt, int substeps )
{
	b3World_Step( world, dt, substeps );
}

// kind: 0 static, 1 kinematic, 2 dynamic.
b3BodyId rf_create_body( b3WorldId world, int32_t kind, b3Vec3 position, b3Quat rotation, uint64_t user )
{
	b3BodyDef def = b3DefaultBodyDef();
	def.type = (b3BodyType)kind;
	def.position = position;
	def.rotation = rotation;
	def.userData = (void*)(uintptr_t)user;
	return b3CreateBody( world, &def );
}

void rf_destroy_body( b3BodyId body )
{
	b3DestroyBody( body );
}

static b3ShapeDef shape_def( const rf_material* material )
{
	b3ShapeDef def = b3DefaultShapeDef();
	def.density = material->density;
	def.baseMaterial.friction = material->friction;
	def.baseMaterial.restitution = material->restitution;
	def.filter.categoryBits = material->category;
	def.filter.maskBits = material->mask;
	return def;
}

bool rf_add_box( b3BodyId body, b3Vec3 half_extents, b3Vec3 offset, const rf_material* material )
{
	b3ShapeDef def = shape_def( material );
	b3BoxHull box = b3MakeOffsetBoxHull( half_extents.x, half_extents.y, half_extents.z, offset );
	return B3_IS_NON_NULL( b3CreateHullShape( body, &def, &box.base ) );
}

bool rf_add_sphere( b3BodyId body, b3Vec3 center, float radius, const rf_material* material )
{
	b3ShapeDef def = shape_def( material );
	b3Sphere sphere = { center, radius };
	return B3_IS_NON_NULL( b3CreateSphereShape( body, &def, &sphere ) );
}

bool rf_add_hull( b3BodyId body, const b3Vec3* points, int32_t count, const rf_material* material )
{
	b3HullData* hull = b3CreateHull( points, count, B3_MAX_HULL_VERTICES );
	if ( hull == NULL )
	{
		return false;
	}
	b3ShapeDef def = shape_def( material );
	// The world keeps its own copy.
	bool created = B3_IS_NON_NULL( b3CreateHullShape( body, &def, hull ) );
	b3DestroyHull( hull );
	return created;
}

bool rf_body_valid( b3BodyId body )
{
	return b3Body_IsValid( body );
}

b3Transform rf_body_transform( b3BodyId body )
{
	return b3Body_GetTransform( body );
}

b3Vec3 rf_body_velocity( b3BodyId body )
{
	return b3Body_GetLinearVelocity( body );
}

b3Vec3 rf_body_angular_velocity( b3BodyId body )
{
	return b3Body_GetAngularVelocity( body );
}

float rf_body_mass( b3BodyId body )
{
	return b3Body_GetMass( body );
}

bool rf_body_awake( b3BodyId body )
{
	return b3Body_IsAwake( body );
}

uint64_t rf_body_user( b3BodyId body )
{
	return (uint64_t)(uintptr_t)b3Body_GetUserData( body );
}

void rf_body_set_user( b3BodyId body, uint64_t user )
{
	b3Body_SetUserData( body, (void*)(uintptr_t)user );
}

void rf_body_set_transform( b3BodyId body, b3Vec3 position, b3Quat rotation )
{
	b3Body_SetTransform( body, position, rotation );
}

void rf_body_set_velocity( b3BodyId body, b3Vec3 linear, b3Vec3 angular )
{
	b3Body_SetLinearVelocity( body, linear );
	b3Body_SetAngularVelocity( body, angular );
}

void rf_body_apply_impulse( b3BodyId body, b3Vec3 impulse, b3Vec3 point )
{
	b3Body_ApplyLinearImpulse( body, impulse, point, true );
}

typedef struct rf_body_list
{
	b3BodyId* bodies;
	int32_t capacity;
	int32_t count;
} rf_body_list;

static bool collect_body( b3ShapeId shape, void* context )
{
	rf_body_list* list = (rf_body_list*)context;
	b3BodyId body = b3Shape_GetBody( shape );
	for ( int32_t i = 0; i < list->count && i < list->capacity; ++i )
	{
		if ( B3_ID_EQUALS( list->bodies[i], body ) )
		{
			return true;
		}
	}
	if ( list->count < list->capacity )
	{
		list->bodies[list->count] = body;
	}
	list->count += 1;
	return true;
}

// The bodies with a shape whose bounds overlap the box and whose category
// is in `mask`. Returns how many there are, which may exceed `capacity`.
int32_t rf_query_box( b3WorldId world, b3Vec3 min, b3Vec3 max, uint64_t mask, b3BodyId* bodies, int32_t capacity )
{
	b3AABB box = { min, max };
	b3QueryFilter filter = b3DefaultQueryFilter();
	filter.maskBits = mask;
	rf_body_list list = { bodies, capacity, 0 };
	b3World_OverlapAABB( world, box, filter, collect_body, &list );
	return list.count;
}
