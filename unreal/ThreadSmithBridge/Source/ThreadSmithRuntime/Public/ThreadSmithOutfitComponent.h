// A skeletal mesh component for ThreadSmith outfits.
// It follows the character's body (leader pose), in the editor and in game,
// so outfits added by the ThreadSmith Bridge keep working after a reload.

#pragma once

#include "CoreMinimal.h"
#include "Components/SkeletalMeshComponent.h"
#include "ThreadSmithOutfitComponent.generated.h"

UCLASS(ClassGroup = (ThreadSmith), meta = (BlueprintSpawnableComponent), hidecategories = (Physics, Collision))
class THREADSMITHRUNTIME_API UThreadSmithOutfitComponent : public USkeletalMeshComponent
{
	GENERATED_BODY()

public:
	UThreadSmithOutfitComponent();

	/** Name of the body component to follow. Empty = follow the component this is attached to. */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "ThreadSmith")
	FName BodyComponentName = TEXT("Body");

	/** Library id of the outfit (from outfit.json), so the Wardrobe can find it again. */
	UPROPERTY(VisibleAnywhere, BlueprintReadOnly, Category = "ThreadSmith")
	FString OutfitId;

	/** Find the body and follow its pose. Called automatically when the component registers. */
	UFUNCTION(BlueprintCallable, Category = "ThreadSmith")
	void FollowBody();

protected:
	virtual void OnRegister() override;
};
