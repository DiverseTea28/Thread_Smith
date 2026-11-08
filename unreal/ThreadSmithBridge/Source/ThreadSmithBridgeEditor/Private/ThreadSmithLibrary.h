// Reads ThreadSmith's outfit library folder (see docs/bridge-protocol.md).

#pragma once

#include "CoreMinimal.h"

/** One outfit folder: outfit.json + outfit.glb + thumbnail.png. */
struct FThreadSmithOutfit
{
	FString Dir;
	FString Id;
	FString Name;
	FString Category;
	FString Character;
	FString MeshFile;
	FString ThumbnailFile;
	FString UnrealSkeleton;
	FString UnrealBody;
	/** Optional: the body with the skin under the clothes removed (body_trimmed.glb). */
	FString TrimmedBodyFile;
	int64 Modified = 0;
	int32 Triangles = 0;

	FString MeshPath() const { return FPaths::Combine(Dir, MeshFile); }
	FString ThumbnailPath() const { return FPaths::Combine(Dir, ThumbnailFile); }
	FString TrimmedBodyPath() const { return TrimmedBodyFile.IsEmpty() ? FString() : FPaths::Combine(Dir, TrimmedBodyFile); }
};

namespace ThreadSmithLibrary
{
	/** The outfit format this plugin understands. Newer folders are skipped. */
	constexpr int32 FormatVersion = 1;

	/** Read one outfit folder. Null if it isn't a valid outfit. */
	TSharedPtr<FThreadSmithOutfit> Read(const FString& OutfitDir);

	/** All outfits in <LibraryFolder>/Outfits, newest first. */
	TArray<TSharedPtr<FThreadSmithOutfit>> List(const FString& LibraryFolder);
}
