#include "ThreadSmithOutfitComponent.h"

#include "GameFramework/Actor.h"

UThreadSmithOutfitComponent::UThreadSmithOutfitComponent()
{
	// The body drives the pose; this component doesn't need its own animation.
	SetCollisionEnabled(ECollisionEnabled::NoCollision);
	bUseBoundsFromLeaderPoseComponent = true;
}

void UThreadSmithOutfitComponent::OnRegister()
{
	Super::OnRegister();
	FollowBody();
}

void UThreadSmithOutfitComponent::FollowBody()
{
	USkeletalMeshComponent* Body = nullptr;

	// 1. A component with the configured name on the same actor.
	if (AActor* Owner = GetOwner(); Owner && !BodyComponentName.IsNone())
	{
		TArray<USkeletalMeshComponent*> Comps;
		Owner->GetComponents<USkeletalMeshComponent>(Comps);
		for (USkeletalMeshComponent* C : Comps)
		{
			if (C != this && C->GetFName() == BodyComponentName)
			{
				Body = C;
				break;
			}
		}
	}
	// 2. Otherwise whatever we're attached to.
	if (!Body)
	{
		Body = Cast<USkeletalMeshComponent>(GetAttachParent());
	}
	if (Body && Body != this)
	{
		SetLeaderPoseComponent(Body, /*bForceUpdate*/ true);
	}
}
