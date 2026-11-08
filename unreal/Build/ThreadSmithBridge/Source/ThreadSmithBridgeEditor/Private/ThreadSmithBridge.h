// The bridge between Unreal and the ThreadSmith app:
// - a local HTTP server (127.0.0.1:47831) that receives outfits,
// - sending bodies to ThreadSmith (127.0.0.1:47830), starting it if needed,
// - importing outfits onto the body skeleton and putting them on characters.

#pragma once

#include "CoreMinimal.h"
#include "Containers/Ticker.h"
#include "HttpResultCallback.h"
#include "HttpRouteHandle.h"
#include "HttpServerRequest.h"

class AActor;
class USkeletalMesh;
class USkeletalMeshComponent;
class USkeleton;
class UBlueprint;
class IHttpRouter;
struct FThreadSmithOutfit;

DECLARE_LOG_CATEGORY_EXTERN(LogThreadSmith, Log, All);

class FThreadSmithBridge
{
public:
	static FThreadSmithBridge& Get();

	void Start();
	void Stop();

	/** Fires when an outfit arrives or the library changes (the Wardrobe refreshes). */
	FSimpleMulticastDelegate OnLibraryChanged;

	// ---- Unreal -> ThreadSmith ----

	/** Send the selected actor's body (and head) to ThreadSmith. */
	void SendSelectionToThreadSmith();

	/** Send a character Blueprint's body (and head) to ThreadSmith. Outfits come back into this Blueprint. */
	void SendBlueprintToThreadSmith(UBlueprint* Blueprint);

	/**
	 * Export this body mesh (and head, if given) and open it in ThreadSmith.
	 * `Actor` / `Blueprint` are remembered so outfits come back to them.
	 */
	void SendBodyToThreadSmith(USkeletalMesh* BodyMesh, AActor* Actor, USkeletalMesh* HeadMesh = nullptr, UBlueprint* Blueprint = nullptr);

	// ---- ThreadSmith -> Unreal ----

	/** Import an outfit's GLB onto the right skeleton (reuses an earlier import). */
	USkeletalMesh* ImportOutfit(const FThreadSmithOutfit& Outfit, bool bForceReimport, FString& OutError);

	/**
	 * Import the outfit's trimmed body (skin under the clothes removed), if it has one.
	 * Null without an error when the outfit has none.
	 */
	USkeletalMesh* ImportTrimmedBody(const FThreadSmithOutfit& Outfit, FString& OutError);

	/** Add the outfit to one placed actor only (used for characters that aren't Blueprints). */
	bool ApplyOutfitToActor(USkeletalMesh* OutfitMesh, const FString& OutfitId, AActor* Actor, FString& OutError, USkeletalMesh* TrimmedBody = nullptr);

	/** Import (if needed) and put an outfit on the selected actor. Used by the Wardrobe. */
	void ApplyOutfitToSelection(const FThreadSmithOutfit& Outfit);

	/**
	 * Add the outfit to a character Blueprint (like MetaTailor does), so every copy of
	 * that character wears it. Replaces the same outfit if it's already there.
	 */
	bool AddOutfitToBlueprint(USkeletalMesh* OutfitMesh, const FString& OutfitId, UBlueprint* Blueprint, FString& OutError, USkeletalMesh* TrimmedBody = nullptr);

	/**
	 * Import (if needed) and add an outfit to the character Blueprint being worked on
	 * (an open Blueprint editor, else the selected character's Blueprint). Used by the Wardrobe.
	 */
	void AddOutfitToTargetBlueprint(const FThreadSmithOutfit& Outfit);

	/** The Blueprint an outfit for `Character` should go into: the one it was sent from, else the open / selected one. */
	UBlueprint* BlueprintForCharacter(const FString& Character) const;

	/** The Blueprint to dress: an open Blueprint editor with a character, else the selected actor's Blueprint. */
	UBlueprint* FindTargetBlueprint() const;

	/** The actor to dress: the one last sent for this character, else the selected one. */
	AActor* FindTargetActor(const FString& Character) const;

	static USkeletalMeshComponent* FindBodyComponent(AActor* Actor);
	static USkeletalMeshComponent* FindFaceComponent(AActor* Actor);

	/** True if ThreadSmith answered recently. */
	bool IsAppConnected() const { return bAppConnected; }

private:
	bool HandleStatus(const FHttpServerRequest& Request, const FHttpResultCallback& OnComplete);
	bool HandleOutfit(const FHttpServerRequest& Request, const FHttpResultCallback& OnComplete);

	USkeleton* ResolveSkeleton(const FString& SkeletonPath, USkeletalMesh* BodyHint) const;
	void PostBody(const FString& JsonBody, int32 AttemptsLeft);
	bool LaunchThreadSmith();
	bool PollAppStatus(float DeltaTime);

	TSharedPtr<IHttpRouter> Router;
	TArray<FHttpRouteHandle> Routes;
	FTSTicker::FDelegateHandle StatusTicker;
	bool bAppConnected = false;
	bool bLaunchedApp = false;

	/** Character name -> the actor it was sent from. */
	TMap<FString, TWeakObjectPtr<AActor>> SentActors;
	/** Character name -> the Blueprint it was sent from (or the sent actor's Blueprint). */
	TMap<FString, TWeakObjectPtr<UBlueprint>> SentBlueprints;
};
